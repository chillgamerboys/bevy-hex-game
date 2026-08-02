//! Pause application adapter. The overlay is rendered by `hex_ui`.

use bevy::prelude::*;
use hex_core::Pause;
use hex_ui::PauseView;

use crate::save::CampaignSaveNotice;
use crate::screens::sandbox::SandboxSession;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, publish_pause_view.run_if(in_state(Pause(true))));
}

fn publish_pause_view(
    sandbox: Option<Res<SandboxSession>>,
    notice: Option<Res<CampaignSaveNotice>>,
    mut view: ResMut<PauseView>,
) {
    let is_sandbox = sandbox.is_some();
    let next = PauseView {
        hint: if is_sandbox {
            "Esc to resume · F5 save unavailable in Sandbox · Backspace to return".to_owned()
        } else {
            "Esc to resume · F5 save exploration · Backspace to Main Menu".to_owned()
        },
        notice: visible_campaign_notice(is_sandbox, notice.as_deref()),
    };
    if *view != next {
        *view = next;
    }
}

fn visible_campaign_notice(
    is_sandbox: bool,
    notice: Option<&CampaignSaveNotice>,
) -> Option<String> {
    (!is_sandbox)
        .then(|| notice.and_then(|notice| notice.0.clone()))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_never_projects_a_campaign_save_notice() {
        let notice = CampaignSaveNotice(Some("Campaign slot 1 saved.".to_owned()));
        assert_eq!(visible_campaign_notice(false, Some(&notice)), notice.0);
        assert_eq!(visible_campaign_notice(true, Some(&notice)), None);
    }
}
