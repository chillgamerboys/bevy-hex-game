//! Pause application adapter. The overlay is rendered by `hex_ui`.

use bevy::prelude::*;
use hex_core::Pause;
use hex_ui::PauseView;

use crate::save::ResumeNotice;
use crate::screens::combat_lab::CombatLabSession;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, publish_pause_view.run_if(in_state(Pause(true))));
}

fn publish_pause_view(
    lab: Option<Res<CombatLabSession>>,
    notice: Option<Res<ResumeNotice>>,
    mut view: ResMut<PauseView>,
) {
    let next = PauseView {
        hint: if lab.is_some() {
            "Esc to resume · F5 save unavailable in Combat Lab · Backspace to return".to_owned()
        } else {
            "Esc to resume · F5 save exploration · Backspace to title".to_owned()
        },
        notice: notice.and_then(|notice| notice.0.clone()),
    };
    if *view != next {
        *view = next;
    }
}
