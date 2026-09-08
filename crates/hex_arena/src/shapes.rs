//! Shape-specific continuous queries. Human capsules retain the M01 kernel.

#[cfg(test)]
#[path = "golem_geometry_tests.rs"]
mod golem_tests;

use crate::collision::{CollisionWorld, Hit, Span, SKIN};
use crate::hex_prisms::{planar_union_exit, HexPrism, HorizontalContact};
use crate::{Actor, BarrierSnapshot, Species};
use bevy_math::{Quat, Vec3};
use hex_core::arena::ArenaVoxelGeometry;
use hex_core::TilePos;

fn axes(yaw: f32) -> [Vec3; 3] {
    let rotation = Quat::from_rotation_y(yaw);
    [rotation * Vec3::X, Vec3::Y, rotation * Vec3::Z]
}

fn box_half(actor: &Actor) -> Vec3 {
    actor.dimensions * 0.5
}

fn prisms(actor: &Actor, feet: Vec3) -> impl Iterator<Item = HexPrism> + '_ {
    actor
        .body_hex_prisms()
        .filter_map(move |part| HexPrism::new(feet + part.offset, part.height))
}

/// Actual face origin for the fixed compound, including its exterior recesses.
/// A locked attack supplies its locked direction independently from live aim.
pub(crate) fn golem_mouth(actor: &Actor, direction: Vec3) -> Vec3 {
    let origin = actor.feet + Vec3::Y * 1.4;
    let direction = direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let exit = planar_union_exit(prisms(actor, actor.feet), origin, direction).unwrap_or(0.0);
    origin + direction * (exit - SKIN).max(0.0)
}

fn closest_body_point(point: Vec3, actor: &Actor) -> Vec3 {
    match actor.species {
        Species::Golem => prisms(actor, actor.feet)
            .map(|part| part.closest_point(point))
            .min_by(|a, b| {
                a.distance_squared(point)
                    .total_cmp(&b.distance_squared(point))
            })
            .unwrap_or(actor.feet),
        Species::Dragon => {
            let rotation = actor.body_rotation();
            let local = rotation.inverse() * (point - actor.center());
            actor.center() + rotation * local.clamp(-box_half(actor), box_half(actor))
        }
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            let radius = actor.dimensions.x * 0.5;
            let axis = Vec3::new(
                actor.feet.x,
                point.y.clamp(
                    actor.feet.y + radius,
                    actor.feet.y + actor.dimensions.y - radius,
                ),
                actor.feet.z,
            );
            axis + (point - axis).normalize_or_zero() * radius.min(point.distance(axis))
        }
    }
}

/// Finds an exposed contact inside the actual body and finite attack cone.
///
/// Try the nearest intersection first. When cover occludes it, seed the same
/// bounded convex intersection from the body's 26 face/edge/corner directions.
/// Capsule samples project onto its rounded surface; a Dragon retains its real
/// oriented box. Each returned contact still needs its own unobstructed attack
/// ray. This conservative finite search never promotes sight of one body part
/// into permission to attack another part through cover.
pub(crate) fn exposed_cone_contact(
    actor: &Actor,
    origin: Vec3,
    direction: Vec3,
    range: f32,
    angle: f32,
    mut exposed: impl FnMut(Vec3) -> bool,
) -> Option<Vec3> {
    if distance(origin, actor) > range + SKIN {
        return None;
    }
    if actor.species == Species::Golem {
        // The body union is nonconvex. Project into one genuine convex prism at
        // a time, never into the union or its bounding box as a convex volume.
        for part in actor.body_hex_prisms() {
            let Some(prism) = HexPrism::new(actor.feet + part.offset, part.height) else {
                continue;
            };
            if prism.distance(origin) > range + SKIN {
                continue;
            }
            let center = actor.feet + part.offset + Vec3::Y * (part.height * 0.5);
            let closest = |point| prism.closest_point(point);
            if let Some(contact) = volume_cone_contact(origin, direction, range, angle, closest) {
                if exposed(contact) {
                    return Some(contact);
                }
            }
            let half = Vec3::new(
                hex_core::config::HEX_SMALL_DIAMETER * 0.5,
                part.height * 0.5,
                1.0,
            );
            for x in [-1_i8, 0, 1] {
                for y in [-1_i8, 0, 1] {
                    for z in [-1_i8, 0, 1] {
                        if x == 0 && y == 0 && z == 0 {
                            continue;
                        }
                        let offset = Vec3::new(f32::from(x), f32::from(y), f32::from(z));
                        let seed = closest(center + offset * half);
                        if let Some(contact) = volume_cone_contact_from_seed(
                            origin, direction, range, angle, seed, closest,
                        ) {
                            if exposed(contact) {
                                return Some(contact);
                            }
                        }
                    }
                }
            }
        }
        return None;
    }
    let closest = |point| closest_body_point(point, actor);
    if let Some(first) = volume_cone_contact(origin, direction, range, angle, closest) {
        if exposed(first) {
            return Some(first);
        }
    }
    let rotation = if actor.species == Species::Dragon {
        actor.body_rotation()
    } else {
        Quat::IDENTITY
    };
    for x in [-1_i8, 0, 1] {
        for y in [-1_i8, 0, 1] {
            for z in [-1_i8, 0, 1] {
                if x == 0 && y == 0 && z == 0 {
                    continue;
                }
                let offset = Vec3::new(f32::from(x), f32::from(y), f32::from(z));
                let seed = closest(actor.center() + rotation * (offset * box_half(actor)));
                if let Some(contact) =
                    volume_cone_contact_from_seed(origin, direction, range, angle, seed, closest)
                {
                    if exposed(contact) {
                        return Some(contact);
                    }
                }
            }
        }
    }
    None
}

/// Bounded intersection of a finite cone and a convex volume's closest-point query.
pub(crate) fn volume_cone_contact(
    origin: Vec3,
    direction: Vec3,
    range: f32,
    angle: f32,
    closest: impl Fn(Vec3) -> Vec3,
) -> Option<Vec3> {
    volume_cone_contact_from_seed(origin, direction, range, angle, closest(origin), closest)
}

fn volume_cone_contact_from_seed(
    origin: Vec3,
    direction: Vec3,
    range: f32,
    angle: f32,
    mut point: Vec3,
    closest: impl Fn(Vec3) -> Vec3,
) -> Option<Vec3> {
    // Alternating projections preserve the finite cone and actual convex body;
    // they also find a flank intersection when the nearest point lies outside.
    for _ in 0..32 {
        let delta = point - origin;
        let distance = delta.length();
        let axial = delta.dot(direction);
        let radial = delta - direction * axial;
        let cone = if distance <= range && (distance < SKIN || axial / distance >= angle.cos()) {
            point
        } else if distance > range && axial / distance >= angle.cos() {
            origin + delta * (range / distance)
        } else {
            let edge = direction * angle.cos() + radial.normalize_or(Vec3::X) * angle.sin();
            origin + edge * delta.dot(edge).clamp(0.0, range)
        };
        let body = closest(cone);
        if body.distance_squared(cone) < SKIN * SKIN {
            return Some(cone);
        }
        point = body;
    }
    None
}

fn hex_support(axis: Vec3) -> f32 {
    [
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
    ]
    .into_iter()
    .map(|p| p.dot(axis).abs())
    .fold(0.0, f32::max)
}

fn box_span_axes(span: Span, feet: Vec3, size: Vec3, yaw: f32) -> Vec<(Vec3, f32, f32)> {
    let half = size * 0.5;
    let [right, _, forward] = axes(yaw);
    let center = feet + Vec3::Y * half.y;
    let local = center - span.coord.to_world((span.bottom + span.top) * 0.5);
    [
        Vec3::X,
        Vec3::new(0.5, 0.0, 0.866_025_4),
        Vec3::new(-0.5, 0.0, 0.866_025_4),
        right,
        forward,
        Vec3::Y,
    ]
    .into_iter()
    .map(|axis| {
        let extent = if axis.y > 0.5 {
            (span.top - span.bottom) * 0.5 + half.y
        } else {
            hex_support(axis) + half.x * right.dot(axis).abs() + half.z * forward.dot(axis).abs()
        };
        (axis, local.dot(axis), extent)
    })
    .collect()
}

fn sweep_axes(
    planes: impl IntoIterator<Item = (Vec3, f32, f32)>,
    delta: Vec3,
    inside_hit: bool,
) -> Option<Hit> {
    let mut enter = -f32::INFINITY;
    let mut exit = f32::INFINITY;
    let mut normal = Vec3::ZERO;
    let mut inside = true;
    for (axis, position, extent) in planes {
        inside &= position.abs() < extent - SKIN;
        let velocity = delta.dot(axis);
        if velocity.abs() <= f32::EPSILON {
            if position <= -extent + SKIN || position >= extent - SKIN {
                return None;
            }
            continue;
        }
        let a = (-extent - position) / velocity;
        let b = (extent - position) / velocity;
        if a.min(b) > enter {
            enter = a.min(b);
            normal = if velocity > 0.0 { -axis } else { axis };
        }
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    if inside && inside_hit {
        return Some(Hit {
            fraction: 0.0,
            normal: Vec3::ZERO,
        });
    }
    if exit < 0.0 || !(-SKIN..=1.0).contains(&enter) || normal.dot(delta) >= 0.0 {
        return None;
    }
    Some(Hit {
        fraction: enter.max(0.0),
        normal,
    })
}

fn box_clear(world: &CollisionWorld, feet: Vec3, size: Vec3, yaw: f32) -> bool {
    feet.is_finite()
        && yaw.is_finite()
        && !world
            .candidates(feet, feet, (size.x * size.x + size.z * size.z).sqrt() * 0.5)
            .any(|span| {
                box_span_axes(span, feet, size, yaw)
                    .into_iter()
                    .all(|(_, p, e)| p.abs() < e - SKIN)
            })
}

pub(crate) fn clear(world: &CollisionWorld, actor: &Actor, feet: Vec3, yaw: f32) -> bool {
    match actor.species {
        Species::Golem => {
            feet.is_finite()
                && actor.body_hex_prisms().all(|part| {
                    world.clear(
                        feet + part.offset,
                        part.height,
                        hex_core::config::HEX_SMALL_DIAMETER * 0.5,
                    )
                })
        }
        Species::Dragon => box_clear(world, feet, actor.dimensions, yaw),
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            world.clear(feet, actor.dimensions.y, actor.dimensions.x * 0.5)
        }
    }
}

pub(crate) fn sweep(world: &CollisionWorld, actor: &Actor, feet: Vec3, delta: Vec3) -> Option<Hit> {
    match actor.species {
        Species::Golem => {
            return actor
                .body_hex_prisms()
                .filter_map(|part| {
                    world.sweep(
                        feet + part.offset,
                        delta,
                        part.height,
                        hex_core::config::HEX_SMALL_DIAMETER * 0.5,
                    )
                })
                .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
        }
        Species::Dragon => {}
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            return world.sweep(feet, delta, actor.dimensions.y, actor.dimensions.x * 0.5);
        }
    }
    let radius =
        (actor.dimensions.x * actor.dimensions.x + actor.dimensions.z * actor.dimensions.z).sqrt()
            * 0.5;
    world
        .candidates(feet, feet + delta, radius)
        .filter_map(|span| {
            sweep_axes(
                box_span_axes(span, feet, actor.dimensions, actor.body_yaw),
                delta,
                false,
            )
        })
        .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
}

pub(crate) fn slide(
    world: &CollisionWorld,
    actor: &Actor,
    mut feet: Vec3,
    mut delta: Vec3,
) -> (Vec3, Vec<Vec3>) {
    let mut contacts = Vec::new();
    for _ in 0..6 {
        if delta.length_squared() < SKIN * SKIN {
            break;
        }
        let Some(hit) = sweep(world, actor, feet, delta) else {
            return (feet + delta, contacts);
        };
        contacts.push(hit.normal);
        feet += delta * hit.fraction + hit.normal * SKIN;
        delta *= 1.0 - hit.fraction;
        delta -= hit.normal * delta.dot(hit.normal).min(0.0);
    }
    (feet, contacts)
}

pub(crate) fn ground(
    world: &CollisionWorld,
    actor: &Actor,
    feet: Vec3,
    distance: f32,
) -> Option<Vec3> {
    let delta = Vec3::NEG_Y * distance;
    sweep(world, actor, feet, delta)
        .filter(|h| h.normal.y > 0.5)
        .map(|h| feet + delta * h.fraction + Vec3::Y * SKIN)
}

/// Test each bounded turn interval with a conservative swept-corner envelope.
/// This prevents a long body rotating through a face despite clear end poses.
pub(crate) fn turn(world: &CollisionWorld, actor: &mut Actor, desired: f32, max_delta: f32) {
    if actor.species == Species::Golem {
        actor.body_yaw = 0.0;
        return;
    }
    let delta = angle_delta(actor.body_yaw, desired).clamp(-max_delta, max_delta);
    let step = delta / 4.0;
    let radius =
        (actor.dimensions.x * actor.dimensions.x + actor.dimensions.z * actor.dimensions.z).sqrt()
            * 0.5;
    for _ in 0..4 {
        let middle = actor.body_yaw + step * 0.5;
        let padding = radius * (step * 0.5).abs();
        let size = actor.dimensions + Vec3::new(padding * 2.0, 0.0, padding * 2.0);
        if !box_clear(world, actor.feet, size, middle) {
            break;
        }
        actor.body_yaw += step;
    }
}

pub(crate) fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

pub(crate) fn distance(point: Vec3, actor: &Actor) -> f32 {
    match actor.species {
        Species::Golem => prisms(actor, actor.feet)
            .map(|part| part.distance(point))
            .fold(f32::INFINITY, f32::min),
        Species::Dragon => {
            let local = actor.body_rotation().inverse() * (point - actor.center());
            (local.abs() - box_half(actor)).max(Vec3::ZERO).length()
        }
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            let radius = actor.dimensions.x * 0.5;
            let low = actor.feet.y + radius;
            let high = actor.feet.y + actor.dimensions.y - radius;
            let axis = Vec3::new(actor.feet.x, point.y.clamp(low, high), actor.feet.z);
            (point.distance(axis) - radius).max(0.0)
        }
    }
}

pub(crate) fn voxel_overlap(pos: TilePos, geometry: ArenaVoxelGeometry, actor: &Actor) -> bool {
    match actor.species {
        Species::Golem => {
            prisms(actor, actor.feet).any(|part| part.overlaps_voxel(pos, geometry, SKIN))
        }
        Species::Dragon => {
            let span = Span {
                coord: pos.coord,
                bottom: geometry.top(pos) - geometry.level_height,
                top: geometry.top(pos),
            };
            box_span_axes(span, actor.feet, actor.dimensions, actor.body_yaw)
                .into_iter()
                .all(|(_, p, e)| p.abs() < e - SKIN)
        }
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            crate::collision::voxel_overlaps_body(
                pos,
                geometry,
                actor.feet,
                actor.dimensions.y,
                actor.dimensions.x * 0.5 + SKIN * 4.0,
            )
        }
    }
}

/// Exact compound overlap with one published vertical hex run, including liquids.
pub(crate) fn hex_span_overlap(
    actor: &Actor,
    coord: hex_core::HexCoord,
    bottom: f32,
    top: f32,
) -> bool {
    let Some(run) = HexPrism::new(coord.to_world(bottom), top - bottom) else {
        return false;
    };
    prisms(actor, actor.feet).any(|part| part.overlap_prism(run, SKIN).is_some())
}

/// Complete compound containment in the existing conservative playable hull.
pub(crate) fn compound_contained(actor: &Actor, geometry: ArenaVoxelGeometry) -> bool {
    let axes = [
        Vec3::Z,
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
    ];
    let limit = f64::from(geometry.radius) * 1.5;
    actor.feet.is_finite()
        && prisms(actor, actor.feet).all(|part| {
            part.horizontal_vertices().all(|point| {
                axes.into_iter()
                    .all(|axis| f64::from(point.dot(axis).abs()) <= limit)
            })
        })
}

fn compound_contact_at(actor: &Actor, feet: Vec3, other: &Actor) -> Option<HorizontalContact> {
    prisms(actor, feet).find_map(|part| match other.species {
        Species::Golem => prisms(other, other.feet).find_map(|b| part.overlap_prism(b, 0.0)),
        Species::Dragon => part.overlap_box(other.feet, other.dimensions, other.body_yaw, 0.0),
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => part
            .overlap_capsule(
                other.feet,
                other.dimensions.y,
                other.dimensions.x * 0.5,
                0.0,
            ),
    })
}

/// A bounded whole-union separation, expressed as each actor's half translation.
/// Internal shared faces are not exterior body boundaries: taking one component's
/// smallest MTV can oscillate forever between neighboring prisms. Bracket a clear
/// whole-body pose with a broad bound, then refine it using only exact component
/// overlap predicates. No AABB establishes contact, and every final move still
/// goes through the existing terrain-aware slide.
pub(crate) fn compound_separation(a: &Actor, b: &Actor) -> Option<Vec3> {
    if a.species != Species::Golem {
        return (b.species == Species::Golem)
            .then(|| compound_separation(b, a).map(|push| -push))
            .flatten();
    }
    let contact = compound_contact_at(a, a.feet, b)?;
    let direction = (a.feet - b.feet).with_y(0.0).normalize_or(contact.normal);
    let mut low = 0.0;
    let mut high =
        (a.dimensions.with_y(0.0).length() + b.dimensions.with_y(0.0).length()) * 0.5 + SKIN * 2.0;
    for _ in 0..18 {
        let middle = (low + high) * 0.5;
        if compound_contact_at(a, a.feet + direction * middle, b).is_some() {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(direction * (high * 0.5 + SKIN))
}

/// Shared actual-body ray/sphere sweep. `predict` freezes the current body pose;
/// live projectiles instead sweep relative to its full latest translation/yaw.
pub(crate) fn sweep_actor(
    start: Vec3,
    delta: Vec3,
    actor: &Actor,
    predict: bool,
    extra: f32,
) -> Option<Hit> {
    if !start.is_finite() || !delta.is_finite() || !extra.is_finite() || extra < 0.0 {
        return None;
    }
    let previous = if predict {
        actor.feet
    } else {
        actor.previous_feet
    };
    let body_delta = actor.feet - previous;
    match actor.species {
        Species::Golem => prisms(actor, previous)
            .filter_map(|part| part.sweep_sphere(start, delta - body_delta, extra))
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
            .map(|hit| Hit {
                fraction: hit.fraction,
                normal: hit.normal,
            }),
        Species::Dragon => sweep_dragon(start, delta, actor, predict, extra).map(|mut hit| {
            // Keep the accepted Dragon impact-normal arithmetic unchanged.
            let point = start + delta * hit.fraction;
            let center = previous + body_delta * hit.fraction + Vec3::Y * actor.dimensions.y * 0.5;
            let local = actor.body_rotation().inverse() * (point - center);
            let nearest =
                center + actor.body_rotation() * local.clamp(-box_half(actor), box_half(actor));
            hit.normal = (point - nearest).normalize_or_zero();
            hit
        }),
        Species::Human | Species::Shadow | Species::Goblin | Species::Shaman => {
            let radius = actor.dimensions.x * 0.5;
            crate::spells::sweep_capsule_dimensions(
                start,
                delta - body_delta,
                previous,
                extra,
                actor.dimensions.y,
                radius,
            )
            .map(|fraction| {
                let point = start + delta * fraction;
                let feet = previous + body_delta * fraction;
                let axis = Vec3::new(
                    feet.x,
                    point
                        .y
                        .clamp(feet.y + radius, feet.y + actor.dimensions.y - radius),
                    feet.z,
                );
                Hit {
                    fraction,
                    normal: (point - axis).normalize_or_zero(),
                }
            })
        }
    }
}

pub(crate) fn sweep_box(
    start: Vec3,
    delta: Vec3,
    center: Vec3,
    half: Vec3,
    yaw: f32,
    extra: f32,
) -> Option<Hit> {
    let basis = axes(yaw);
    let planes = basis
        .into_iter()
        .zip([half.x, half.y, half.z])
        .map(|(axis, e)| (axis, (start - center).dot(axis), e + extra));
    sweep_axes(planes, delta, true)
}

pub(crate) fn sweep_barrier(
    barrier: &BarrierSnapshot,
    start: Vec3,
    delta: Vec3,
    radius: f32,
) -> Option<Hit> {
    sweep_box(
        start,
        delta,
        barrier.center,
        Vec3::new(barrier.width * 0.5, barrier.height * 0.5, 0.025),
        (-barrier.normal.x).atan2(-barrier.normal.z),
        radius,
    )
}

pub(crate) fn sweep_dragon(
    start: Vec3,
    delta: Vec3,
    actor: &Actor,
    predict: bool,
    extra: f32,
) -> Option<Hit> {
    let previous = if predict {
        actor.feet
    } else {
        actor.previous_feet
    };
    let previous_yaw = if predict {
        actor.body_yaw
    } else {
        actor.previous_yaw
    };
    let angular = angle_delta(previous_yaw, actor.body_yaw);
    let body_delta = actor.feet - previous;
    let half = box_half(actor);
    let mut result = None;
    // Real body yaw is capped before translation. Eight chronological slices
    // include its corner arc, including fast crossing projectiles.
    for i in 0..8u16 {
        let t = f32::from(i) / 8.0;
        let mid = t + 0.0625;
        let center = previous + body_delta * t + Vec3::Y * half.y;
        let yaw = previous_yaw + angular * mid;
        let padding = Vec3::new(half.x, 0.0, half.z).length() * angular.abs() / 16.0;
        if let Some(mut hit) = sweep_box(
            start + delta * t,
            (delta - body_delta) / 8.0,
            center,
            half,
            yaw,
            extra + padding,
        ) {
            hit.fraction = t + hit.fraction / 8.0;
            result = Some(hit);
            break;
        }
    }
    result
}
