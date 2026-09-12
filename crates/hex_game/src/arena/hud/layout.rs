//! Responsive Battle UI tree. Gameplay labels retain their authoritative adapter.
use super::*;
use crate::arena::ux::{self, Page, UxAction, UxLabel};

fn column() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(14),
        min_width: px(0),
        ..default()
    }
}
fn button(parent: &mut ChildSpawnerCommands, label: &str, action: impl Bundle) -> Entity {
    parent
        .spawn((
            Button,
            BorderColor::all(Color::NONE),
            Node {
                border: UiRect::all(px(2)),
                min_width: px(100),
                min_height: px(52),
                padding: UiRect::axes(px(18), px(10)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(6)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.24, 0.29)),
            action,
        ))
        .with_children(|b| {
            b.spawn(text(label, 26.0, INK));
        })
        .id()
}
fn row() -> Node {
    Node {
        flex_wrap: FlexWrap::Wrap,
        column_gap: px(12),
        row_gap: px(10),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    }
}
fn scroll() -> impl Bundle {
    (
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            ..column()
        },
        ScrollPosition::default(),
        ux::MenuScroll,
    )
}
fn panel() -> Node {
    Node {
        width: px(1120),
        max_width: percent(94),
        height: percent(92),
        padding: UiRect::all(px(24)),
        border_radius: BorderRadius::all(px(12)),
        ..column()
    }
}
fn overlay() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    }
}

pub(crate) fn setup(mut commands: Commands, mut images: Option<ResMut<Assets<Image>>>) {
    let icons: [Handle<Image>; 3] = std::array::from_fn(|i| {
        images
            .as_mut()
            .map_or_else(Handle::default, |images| ux::icons::create(images, i))
    });
    commands.init_resource::<ux::UxState>();
    commands
        .spawn((overlay(), GlobalZIndex(10), CombatHud))
        .with_children(|root| {
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: percent(50),
                    left: percent(50),
                    margin: UiRect::axes(px(-9), px(-18)),
                    ..default()
                },
                text("+", 32.0, INK),
                ux::Reticle,
                TextShadow {
                    offset: Vec2::splat(1.5),
                    color: Color::BLACK,
                },
            ));
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: percent(50),
                    left: percent(50),
                    margin: UiRect::new(px(-180), px(0), px(30), px(0)),
                    width: px(360),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                text("", 22.0, INK),
                UxLabel::Target,
                TextShadow {
                    offset: Vec2::splat(1.5),
                    color: Color::BLACK,
                },
            ));
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: px(24),
                    width: percent(100),
                    align_items: AlignItems::Center,
                    ..column()
                },
                Name::new("Combat strip"),
            ))
            .with_children(|strip| {
                strip.spawn((
                    Node {
                        max_width: percent(80),
                        padding: UiRect::axes(px(14), px(6)),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    text("", 24.0, INK),
                    UxLabel::Notice,
                ));
                strip.spawn((
                    Node {
                        padding: UiRect::axes(px(14), px(5)),
                        border_radius: BorderRadius::all(px(6)),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    text("", 24.0, INK),
                    Label::Health,
                ));
                strip.spawn(row()).with_children(|bar| {
                    for (index, (key, name, _icon)) in [
                        ("RMB", "SHIELD", "[]"),
                        ("LMB", "FIREBALL", "*"),
                        ("E", "HIGH JUMP", "↑"),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        bar.spawn((
                            Node {
                                width: px(220),
                                min_height: px(100),
                                padding: UiRect::all(px(12)),
                                border: UiRect::all(px(2)),
                                border_radius: BorderRadius::all(px(8)),
                                row_gap: px(5),
                                flex_direction: FlexDirection::Column,
                                ..default()
                            },
                            BackgroundColor(PANEL),
                            BorderColor::all(MUTED),
                            SpellCard(index),
                        ))
                        .with_children(|card| {
                            card.spawn(Node {
                                align_items: AlignItems::Center,
                                column_gap: px(12),
                                ..default()
                            })
                            .with_children(|r| {
                                r.spawn((
                                    Node {
                                        width: px(44),
                                        height: px(44),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    ImageNode::new(icons.get(index).cloned().unwrap_or_default()),
                                    ux::SpellIcon(index),
                                ));
                                r.spawn(text(format!("{key}  {name}"), 22.0, INK));
                            });
                            card.spawn((text("", 26.0, INK), Label::Spell(index)));
                            card.spawn((
                                Node {
                                    width: percent(100),
                                    height: px(8),
                                    border_radius: BorderRadius::all(px(4)),
                                    overflow: Overflow::clip(),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.13, 0.19, 0.23)),
                            ))
                            .with_children(|track| {
                                track.spawn((
                                    Node {
                                        width: percent(100),
                                        height: percent(100),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.4, 0.9, 0.8)),
                                    ux::SpellFill(index),
                                ));
                            });
                        });
                    }
                });
            });
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: px(24),
                    top: px(24),
                    width: px(300),
                    padding: UiRect::all(px(10)),
                    display: Display::None,
                    ..column()
                },
                BackgroundColor(PANEL),
                ux::MiniMap,
            ))
            .with_children(|map| {
                map.spawn(text("N ↑   MAP  /  M", 20.0, INK));
                ux::spawn_map(map, false, 280.0);
                map.spawn((text("", 18.0, INK), UxLabel::Pin));
            });
        });
    // Recording status remains visible over menus as well as gameplay.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(16),
            padding: UiRect::axes(px(10), px(5)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(PANEL),
        GlobalZIndex(40),
        text("", 20.0, Color::srgb(1.0, 0.36, 0.32)),
        UxLabel::Recording,
    ));
    commands
        .spawn((overlay(), GlobalZIndex(10), ObserverHud))
        .with_children(|r| {
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: px(18),
                    left: px(18),
                    padding: UiRect::all(px(12)),
                    ..default()
                },
                BackgroundColor(PANEL),
                text("", 22.0, INK),
                Label::ObserverTeams,
            ));
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: px(18),
                    right: px(18),
                    max_width: px(500),
                    padding: UiRect::all(px(12)),
                    ..default()
                },
                BackgroundColor(PANEL),
                text("", 22.0, INK),
                Label::ObserverStatus,
            ));
            r.spawn((Node { position_type:PositionType::Absolute,bottom:px(14),width:percent(100),padding:UiRect::horizontal(px(18)),justify_content:JustifyContent::Center,..default() },Name::new("Observer footer container"))).with_children(|footer| {
                footer.spawn((Node {padding:UiRect::axes(px(12),px(6)),border_radius:BorderRadius::all(px(4)),max_width:percent(100),..default()},BackgroundColor(PANEL),Name::new("Observer footer panel"))).with_children(|panel| {
                    panel.spawn((text("WASD move   Q / E vertical   Shift fast   Mouse look   Wheel zoom   C orbit / free   Esc pause",18.0,INK),Name::new("Observer footer text")));
                });
            });
        });
    commands.spawn((overlay(),BackgroundColor(Color::srgba(0.01,0.02,0.035,0.8)),GlobalZIndex(20),StartPanel)).with_children(|overlay| {
        overlay.spawn((panel(),BackgroundColor(PANEL),ux::MenuPanel)).with_children(|p| {
            p.spawn(text("BATTLE MODE",36.0,INK));
            p.spawn(scroll()).with_children(|p| {
                p.spawn(row()).with_children(|r| {
                    button(r,"PLAY",Action::Control(ArenaControl::Player));
                    button(r,"SPECTATE BATTLE",Action::Control(ArenaControl::Spectator));
                });
                p.spawn(text("Choose your map",26.0,INK));
                p.spawn(row()).with_children(|r| {
                    for map in [ArenaMap::ForestMassif,ArenaMap::Duel,ArenaMap::Fort,ArenaMap::SevenRegions] { button(r,crate::arena::map_name(map),Action::Map(map)); }
                });
                p.spawn((column(),ModeContent(ArenaControl::Player))).with_children(|p| {
                    p.spawn(text("Enemy party",26.0,INK));
                    p.spawn(row()).with_children(|r| {
                        for (name,action) in [("Dragon",Action::Encounter(ArenaEncounter::Dragon)),("Goblins",Action::Encounter(ArenaEncounter::Goblins)),("Shaman party",Action::Encounter(ArenaEncounter::ShamanParty)),("Shadow",Action::Encounter(ArenaEncounter::Shadow)),("Golem",Action::PlayerRecipe(BattlePreset::Golem)),("Wisps",Action::PlayerRecipe(BattlePreset::Wisps4)),("Worm",Action::PlayerRecipe(BattlePreset::Worm))] {button(r,name,action);}
                    });
                });
                p.spawn((column(),ModeContent(ArenaControl::Spectator))).with_children(|p| {
                    for slot in 0..2 { p.spawn(row()).with_children(|r| {
                        r.spawn((text("",26.0,INK),Label::Team(slot)));
                        button(r,"Previous",Action::Roster(slot,-1));button(r,"Next",Action::Roster(slot,1));
                    }); }
                });
                p.spawn((text("",26.0,INK),Label::Selection));
                p.spawn((text("",24.0,INK),Label::Help));
                p.spawn(text("M toggles your map. Esc opens upgrades, settings and recording.\nCombat waits until you start.",24.0,INK));
            });
            // Fixed action positions: Restart reopens a safe Start button.
            p.spawn(row()).with_children(|r| {
                button(r,"FULLSCREEN",Action::Fullscreen);
                button(r,"UI SIZE",UxAction::Scale);
                button(r,"QUIT GAME",Action::Quit);
            });
            p.spawn((Button,BorderColor::all(Color::NONE),Node { border:UiRect::all(px(2)), width:percent(100),min_height:px(58),align_items:AlignItems::Center,justify_content:JustifyContent::Center,flex_shrink:0.0,..default() },BackgroundColor(Color::srgb(0.16,0.37,0.41)),Action::Start)).with_children(|r|{r.spawn(text("START  /  ENTER",28.0,INK));});
        });
    });
    commands.spawn((Node { display:Display::None,..overlay() },BackgroundColor(Color::srgba(0.01,0.02,0.035,0.8)),GlobalZIndex(20),PausePanel)).with_children(|overlay| {
        overlay.spawn((panel(),BackgroundColor(PANEL),ux::MenuPanel)).with_children(|p| {
            p.spawn((text("",36.0,INK),Label::MenuTitle));
            p.spawn((text("",24.0,INK),Label::MenuHelp,ux::MenuHelper));
            p.spawn(row()).with_children(|r| { for page in Page::ALL {button(r,page.name(),UxAction::Page(page));} });
            p.spawn(scroll()).with_children(|body| {
                for page in Page::ALL {
                    body.spawn((Node { display:if page==Page::Overview {Display::Flex}else{Display::None},..column() },ux::PageBody(page))).with_children(|p| {
                        match page {
                            Page::Overview => {
                                p.spawn((text("",28.0,INK),Label::Health));
                                p.spawn((text("",26.0,INK),Label::Encounter));
                                p.spawn((text("",26.0,INK),Label::Rewards));

                                p.spawn((text("",26.0,INK),Label::MenuRules));
                                p.spawn((text("",24.0,INK),Label::Status));
                                p.spawn((text("",24.0,INK),UxLabel::RecorderFull));
                            }
                            Page::Map => {
                                ux::spawn_map(p,true,480.0);
                                p.spawn((text("",24.0,INK),UxLabel::MapSelection));
                                p.spawn(text("D  Dragon     S  Shadow     T  Troll\n+  Charged fountain     ○  Spent fountain     ×  Defeated\nClick a marker to inspect it. Click terrain to place your destination.",22.0,INK));
                                button(p,"CLEAR DESTINATION",UxAction::ClearPin);
                            }
                            Page::Upgrades => {
                                p.spawn(text("Each level earns one upgrade point. Disabled upgrades cost nothing.",26.0,INK));
                                for index in 0..12 { p.spawn(Node { min_height:px(58),width:percent(100),align_items:AlignItems::Center,column_gap:px(10),flex_shrink:0.0,..default() }).with_children(|r| {
                                    r.spawn((Node {flex_grow:1.0,flex_basis:px(0),min_width:px(0),..default()},text("",26.0,INK),Label::Parameter(index)));
                                    button(r,"−",Action::Change(index,-1.0));button(r,"+",Action::Change(index,1.0));
                                }); }
                            }
                            Page::Settings => {
                                p.spawn((text("",26.0,INK),UxLabel::Scale));
                                button(p,"CHANGE UI SIZE",UxAction::Scale);
                                p.spawn((Button, Node { min_height:px(52),padding:UiRect::axes(px(18),px(10)),border:UiRect::all(px(2)),align_items:AlignItems::Center,justify_content:JustifyContent::Center,flex_shrink:0.0,..default() }, BackgroundColor(Color::srgb(0.12, 0.24, 0.29)), BorderColor::all(Color::NONE), Action::Fullscreen)).with_children(|b| {b.spawn((text("Fullscreen",26.0,INK),Label::WindowMode));});
                                p.spawn(text("C switches first / third person.\nUI preferences persist; Restart resets only your run.",26.0,INK));
                            }
                            Page::Controls => {p.spawn(text("WASD   Move\nMouse   Look\nSpace   Jump\nE   High Jump\nHold LMB / release   Charge / cast Fireball\nHold RMB / release   Charge / cast Shield\nC   First / third person\nT   Trajectory preview\nM   Toggle minimap\nEsc / Tab   Pause / resume\nR   Restart run\nF9   Bookmark a recording\nMenus: arrows select, Enter activates, wheel scrolls",26.0,INK));}
                        }
                    });
                }
            });
            p.spawn(row()).with_children(|r| {
                r.spawn((Button,BorderColor::all(Color::NONE),Node { border:UiRect::all(px(2)), min_height:px(52),padding:UiRect::axes(px(18),px(10)),align_items:AlignItems::Center,justify_content:JustifyContent::Center,..default() },BackgroundColor(Color::srgb(0.12,0.24,0.29)),UxAction::Record)).with_children(|b|{b.spawn((text("START RECORDING",26.0,INK),UxLabel::RecordButton));});
                button(r,"OPEN RECORDINGS",UxAction::OpenFolder);
                button(r,"QUIT GAME",Action::Quit);
            });
            p.spawn((text("",20.0,INK),UxLabel::RecorderDetail));
            p.spawn(Node {width:percent(100),column_gap:px(12),flex_shrink:0.0,..default()}).with_children(|r| {
                for (name,action) in [("RESUME",Action::Resume),("RESTART",Action::Restart)] {
                    r.spawn((Button,BorderColor::all(Color::NONE),Node { border:UiRect::all(px(2)),flex_grow:1.0,flex_basis:px(0),min_width:px(0),min_height:px(58),align_items:AlignItems::Center,justify_content:JustifyContent::Center,..default()},BackgroundColor(Color::srgb(0.16,0.37,0.41)),action)).with_children(|b|{
                        let mut label=b.spawn(text(name,26.0,INK));if matches!(action,Action::Resume){label.insert(Label::Resume);}
                    });
                }
            });
        });
    });
}
