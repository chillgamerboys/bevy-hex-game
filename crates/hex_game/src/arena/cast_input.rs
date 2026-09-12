//! Native gestures stay spell-tagged until the fixed simulation consumes them.
use super::*;
use bevy::input::{mouse::MouseButtonInput, ButtonState};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Edge {
    spell: Spell,
    pressed: bool,
    released: bool,
    aim: Vec3,
}

pub(super) struct CastInput {
    pub managed: bool,
    owner: Option<Spell>,
    latched: [bool; 2],
    queue: VecDeque<Edge>,
    last_used: Spell,
    aim: Vec3,
}

impl Default for CastInput {
    fn default() -> Self {
        Self {
            managed: false,
            owner: None,
            latched: [false; 2],
            queue: VecDeque::new(),
            last_used: Spell::Fireball,
            aim: Vec3::NEG_Z,
        }
    }
}

impl CastInput {
    pub fn clear(&mut self) {
        self.owner = None;
        self.latched = [false; 2];
        self.queue.clear();
    }

    pub fn spell(&self) -> Spell {
        self.owner.unwrap_or(self.last_used)
    }

    fn press(&mut self, spell: Spell, aim: Vec3) {
        let Some(latched) = self.latched.get_mut(spell.index()) else {
            return;
        };
        if *latched {
            return;
        }
        *latched = true;
        // Ignored/overflowed presses must be released and freshly pressed later.
        if self.owner.is_some() || self.queue.len() >= 32 {
            return;
        }
        self.owner = Some(spell);
        self.last_used = spell;
        self.queue.push_back(Edge {
            spell,
            pressed: true,
            released: false,
            aim,
        });
    }

    fn release(&mut self, spell: Spell, aim: Vec3) {
        if let Some(latched) = self.latched.get_mut(spell.index()) {
            *latched = false;
        }
        if self.owner != Some(spell) {
            return;
        }
        self.owner = None;
        if let Some(edge) = self
            .queue
            .back_mut()
            .filter(|edge| edge.spell == spell && !edge.released)
        {
            edge.released = true;
            edge.aim = aim;
        } else {
            self.queue.push_back(Edge {
                spell,
                pressed: false,
                released: true,
                aim,
            });
        }
    }

    pub fn observe(
        &mut self,
        mouse: &ButtonInput<MouseButton>,
        events: &[MouseButtonInput],
        aim: Vec3,
    ) {
        self.managed = true;
        self.aim = aim;
        if events.is_empty() {
            // Synthetic/native snapshots without event order use Fireball first.
            for (button, spell) in [
                (MouseButton::Left, Spell::Fireball),
                (MouseButton::Right, Spell::Shield),
            ] {
                if !mouse.pressed(button) && !mouse.just_pressed(button) {
                    self.release(spell, aim);
                }
            }
            for (button, spell) in [
                (MouseButton::Left, Spell::Fireball),
                (MouseButton::Right, Spell::Shield),
            ] {
                if mouse.just_pressed(button) {
                    self.press(spell, aim);
                }
                if mouse.just_released(button) {
                    self.release(spell, aim);
                }
            }
        } else {
            for event in events {
                let spell = match event.button {
                    MouseButton::Left => Spell::Fireball,
                    MouseButton::Right => Spell::Shield,
                    _ => continue,
                };
                match event.state {
                    ButtonState::Pressed => self.press(spell, aim),
                    ButtonState::Released => self.release(spell, aim),
                }
            }
        }
        if let Some(edge) = self.queue.back_mut().filter(|edge| !edge.released) {
            edge.aim = aim;
        }
    }

    pub fn write(&self, intent: &mut ActorIntent) {
        if let Some(edge) = self.queue.front() {
            intent.aim = self.aim;
            intent.selected = Some(edge.spell);
            intent.cast_pressed = edge.pressed;
            intent.cast_released = edge.released;
            intent.cast_held = !edge.released;
            if edge.released {
                intent.aim = edge.aim;
            }
        } else {
            if self.owner.is_some() {
                intent.aim = self.aim;
            }
            intent.selected = Some(self.spell());
            intent.cast_pressed = false;
            intent.cast_released = false;
            intent.cast_held = self.owner.is_some();
        }
    }

    pub fn consume(&mut self) {
        self.queue.pop_front();
    }
}
