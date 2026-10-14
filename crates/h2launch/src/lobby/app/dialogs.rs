//! Halo 2's dialogs (menu.md, "What Phase 2.8 built"): every popup but
//! ADD FRIEND, the party invitation and the offline screen, drawn by
//! `h2ui` from their tags (the quarter dialogs ok_cancel and
//! multiple_choice, the half one large_error, with their frame pieces
//! ul_07, br_07, dialog1 and dialog2) over the globals' overlay, each a
//! list of rows the focus moves between. An invitation also shows Halo
//! 2's notice at the lower left (the beeper) with the gamertag. Without
//! the tags they are `h2ui`'s built-in ones in the flat look.
//!
//! What each row does is what a press did before the dialogs (`press`):
//! the row today's A did has the focus when a dialog opens, so A still
//! does it, B still cancels or declines, and the shortcuts (Quit's Y,
//! the offline screen's X and Y) stay.

use super::{App, Area, Canvas, FriendOption, Hit, Input, Popup, Screen};
use h2ui::anim::{Focus, ItemLook, PULSE_HIGH, PULSE_LOW};
use h2ui::catalog::ScreenId;
use h2ui::layout::{Face, LegendItem, ScreenLayout, SlotText, Space};
use h2ui::paint::{DrawList, Resources};
use h2ui::screens::{self, ListState, Row, ScreenState, Show, UiHit, Words};
use std::borrow::Cow;

/// What picking a dialog's row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogAct {
    Quit,
    SignOut,
    Cancel,
    Leave,
    Remove,
    Unfriend,
    Option(FriendOption),
    Accept,
    Decline,
    TryAgain,
    Controls,
    SignIn,
    MainMenu,
}

impl DialogAct {
    /// The press that did it before the dialogs had rows (and still does
    /// from its shortcut): A for the row A did, B for cancelling or
    /// declining, Quit's Y for signing out, the offline screen's X and Y.
    /// A friend's option is A with that option selected.
    pub(super) fn press(self) -> Input {
        match self {
            DialogAct::Quit
            | DialogAct::Leave
            | DialogAct::Remove
            | DialogAct::Unfriend
            | DialogAct::Option(_)
            | DialogAct::Accept
            | DialogAct::TryAgain => Input::A,
            DialogAct::Cancel | DialogAct::Decline | DialogAct::MainMenu => Input::B,
            DialogAct::SignOut | DialogAct::SignIn => Input::Y,
            DialogAct::Controls => Input::X,
        }
    }
}

/// A dialog on show: which catalog screen draws it, its header and body
/// (our own words), its rows and what each does, the focused row, and its
/// button legend.
pub(super) struct DialogView {
    pub id: ScreenId,
    pub header: String,
    pub body: String,
    pub rows: Vec<(String, DialogAct)>,
    pub focus: usize,
    pub legend: Vec<LegendItem>,
}

/// Which dialog is up: a popup, the invitation from a party, or the
/// offline screen. A new one opens with its first row focused and its
/// pieces coming in from then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogKey {
    Popup(Popup),
    Invite(u64),
    Failed,
}

/// A dialog's button legend by its tag's `button_key` (the globals'
/// legend names, read here as their buttons and our short words; Vista's
/// keyboard legends are mangled in the tags, menu.md 9.2): 1 A SELECT B
/// BACK, 2 A SELECT B CANCEL, 3 A ENTER B CANCEL, 9 A SELECT, 0xD B
/// CANCEL, 0x16 A OK; none for the others.
pub(super) fn legend_of(button_key: u16) -> &'static [(Face, &'static str)] {
    match button_key {
        1 => &[(Face::A, "SELECT"), (Face::B, "BACK")],
        2 => &[(Face::A, "SELECT"), (Face::B, "CANCEL")],
        3 => &[(Face::A, "ENTER"), (Face::B, "CANCEL")],
        9 => &[(Face::A, "SELECT")],
        0xD => &[(Face::B, "CANCEL")],
        0x16 => &[(Face::A, "OK")],
        _ => &[],
    }
}

/// The beeper's own pulse: the globals' animation 48 its pieces have
/// gives an ambient fade from 1 to 0.2 over 500 ms; how it comes back up
/// is our estimate (over the next 500 ms), so a whole pulse is a second.
const BEEPER_PULSE: f64 = 1.0;

/// How opaque the beeper is `t` seconds after it showed: full at 0,
/// faintest half a pulse later.
pub(super) fn beeper_pulse(t: f64) -> f32 {
    let phase = (t / BEEPER_PULSE).rem_euclid(1.0) as f32;
    let wave = 0.5 + 0.5 * (phase * std::f32::consts::TAU).cos();
    PULSE_LOW + (PULSE_HIGH - PULSE_LOW) * wave
}

/// The words of the invitation's notice: the tag's for its upper line
/// (ours without them), the gamertag on the lower.
const BEEPER_WORDS: &str = "INVITATION";

impl App {
    /// The dialog up now, if any.
    pub(super) fn dialog_key(&self) -> Option<DialogKey> {
        if let Some(p) = self.popup {
            return Some(DialogKey::Popup(p));
        }
        if let Some((party, _)) = self.invite() {
            return Some(DialogKey::Invite(party));
        }
        (self.screen == Screen::Failed).then_some(DialogKey::Failed)
    }

    /// A popup or the invitation over a screen (not the offline screen,
    /// which is a screen of its own): what a double-click's halves are
    /// told apart by.
    pub(super) fn over_key(&self) -> Option<DialogKey> {
        self.dialog_key().filter(|k| *k != DialogKey::Failed)
    }

    /// Notice a dialog opening: its first row (or the friend's option
    /// selected) has the focus, and its pieces come in from now.
    pub(super) fn follow_dialog(&mut self) {
        let key = self.dialog_key();
        if key == self.dialog_shown {
            return;
        }
        self.dialog_shown = key;
        let now = self.menu_clock();
        self.dialog_opened = now;
        let first = match key {
            Some(DialogKey::Popup(Popup::FriendOptions(_))) => self.osel,
            _ => 0,
        };
        self.dialog_focus = Focus::on(first, now);
    }

    /// The layout of catalog screen `id`: the tags', else `h2ui`'s
    /// built-in one.
    pub(super) fn dialog_layout(&self, id: ScreenId) -> Option<Cow<'_, ScreenLayout>> {
        self.cfg.menus.shell.screen_or_built_in(id)
    }

    /// The dialog up now as Halo 2 draws it: none for ADD FRIEND (its
    /// keyboard comes in 2.14) or when nothing is up.
    pub(super) fn dialog_view(&self) -> Option<DialogView> {
        use DialogAct as D;
        let cancel = ("CANCEL".to_string(), D::Cancel);
        let (id, header, body, rows) = match self.dialog_key()? {
            DialogKey::Popup(Popup::Quit) => {
                let mut rows = vec![("QUIT".to_string(), D::Quit)];
                if self.client.is_some() {
                    rows.push(("SIGN OUT".to_string(), D::SignOut));
                }
                rows.push(cancel);
                let body = "Close the launcher?".to_string();
                (ScreenId::MultipleChoice, "QUIT".to_string(), body, rows)
            }
            DialogKey::Popup(Popup::LeaveGame) => (
                ScreenId::OkCancel,
                "LEAVE GAME".to_string(),
                "Leave the game in progress? In a ranked playlist it counts as a loss.".to_string(),
                vec![("LEAVE GAME".to_string(), D::Leave), cancel],
            ),
            DialogKey::Popup(Popup::Kick(who)) => {
                let name = self
                    .party()
                    .and_then(|pt| pt.members.iter().find(|m| m.account == who))
                    .map_or("this player".into(), |m| m.gamertag.clone());
                (
                    ScreenId::OkCancel,
                    "REMOVE PLAYER".to_string(),
                    format!("Remove {name} from your party? They can come back only if invited."),
                    vec![("REMOVE".to_string(), D::Remove), cancel],
                )
            }
            DialogKey::Popup(Popup::Unfriend(who)) => {
                let name = self
                    .friend(who)
                    .map_or("this player".into(), |f| f.gamertag.clone());
                (
                    ScreenId::OkCancel,
                    "REMOVE FRIEND".to_string(),
                    format!("Remove {name} from your friends? They won't be told."),
                    vec![("REMOVE FRIEND".to_string(), D::Unfriend), cancel],
                )
            }
            DialogKey::Popup(Popup::FriendOptions(who)) => {
                let f = self.friend(who);
                let name = f.map_or(String::new(), |f| f.gamertag.clone());
                let status = f.map_or(String::new(), |f| self.friend_status(f).0);
                let rows = self
                    .friend_options(who)
                    .into_iter()
                    .map(|o| (o.label().to_string(), D::Option(o)))
                    .collect();
                (ScreenId::MultipleChoice, name, status, rows)
            }
            DialogKey::Popup(Popup::AddFriend) => return None,
            DialogKey::Invite(_) => {
                let (_, from) = self.invite()?;
                (
                    ScreenId::MultipleChoice,
                    "PARTY INVITE".to_string(),
                    format!("{from} invited you to their party."),
                    vec![
                        ("ACCEPT".to_string(), D::Accept),
                        ("DECLINE".to_string(), D::Decline),
                    ],
                )
            }
            DialogKey::Failed => {
                let body = format!(
                    "{}\n\nServer: {}\nGamertag: {}",
                    self.failed, self.settings.server, self.settings.gamertag
                );
                let rows = vec![
                    ("TRY AGAIN".to_string(), D::TryAgain),
                    ("CONTROLLER SETTINGS".to_string(), D::Controls),
                    ("SIGN IN".to_string(), D::SignIn),
                    ("MAIN MENU".to_string(), D::MainMenu),
                ];
                (ScreenId::LargeError, "OFFLINE".to_string(), body, rows)
            }
        };
        let focus = match self.popup {
            Some(Popup::FriendOptions(_)) => self.osel,
            _ => self.dialog_focus.item,
        };
        let focus = focus.min(rows.len().saturating_sub(1));
        let button_key = self.dialog_layout(id).map_or(2, |l| l.button_key);
        let legend = legend_of(button_key)
            .iter()
            .map(|&(face, label)| {
                let pressable = super::shell::input_of(face).is_some_and(|i| !self.unpressable(i));
                match pressable {
                    true => LegendItem::new(face, label),
                    false => LegendItem::label_only(face, label),
                }
            })
            .collect();
        Some(DialogView {
            id,
            header,
            body,
            rows,
            focus,
            legend,
        })
    }

    /// The focus fades of the dialog up now (its list skin's), or the
    /// default skin's.
    fn dialog_look(&self) -> ItemLook {
        let id = self.dialog_view().map(|v| v.id);
        let layout = id.and_then(|id| self.dialog_layout(id));
        layout
            .and_then(|l| l.pane(0).and_then(|p| p.lists.first()).map(|l| l.skin.items.clone()))
            .unwrap_or_default()
    }

    /// The dialog's focus moved to row `k`, fading as its skin does.
    pub(super) fn focus_dialog(&mut self, k: usize) {
        let look = self.dialog_look();
        self.dialog_focus = self.dialog_focus.moved_in(&look, k, self.menu_clock());
    }

    /// Up or Down in a dialog: the focus moves a row, from the last to
    /// the first and back.
    pub(super) fn move_dialog(&mut self, down: bool) {
        let Some(v) = self.dialog_view() else {
            return;
        };
        let n = v.rows.len().max(1);
        let k = match down {
            true => (v.focus + 1) % n,
            false => (v.focus + n - 1) % n,
        };
        self.focus_dialog(k);
    }

    /// What the focused row of the dialog up now does.
    pub(super) fn focused_act(&self) -> Option<DialogAct> {
        let v = self.dialog_view()?;
        v.rows.get(v.focus).map(|r| r.1)
    }

    /// A press in a popup: Up and Down move between the rows of a dialog
    /// with rows of its own, A does the focused row's press; the rest, and
    /// the friend's options (which move their own selection) and ADD
    /// FRIEND, go as before (`popup_press`).
    pub(super) fn popup_input(&mut self, p: Popup, i: Input) {
        let rows = matches!(
            p,
            Popup::Quit | Popup::LeaveGame | Popup::Kick(_) | Popup::Unfriend(_)
        );
        if rows {
            match i {
                Input::Up | Input::Down => return self.move_dialog(i == Input::Down),
                Input::A => {
                    if let Some(act) = self.focused_act() {
                        return self.popup_press(p, act.press());
                    }
                }
                _ => {}
            }
        }
        self.popup_press(p, i)
    }

    /// A press while the invitation from `party` (`from`'s) is up: it
    /// takes every press. Up and Down move between ACCEPT and DECLINE, A
    /// does the focused one, B declines.
    pub(super) fn invite_input(&mut self, party: u64, from: &str, i: Input) {
        match i {
            Input::Up | Input::Down => self.move_dialog(i == Input::Down),
            Input::A => {
                let press = self.focused_act().map_or(Input::A, DialogAct::press);
                self.invite_press(party, from, press);
            }
            _ => self.invite_press(party, from, i),
        }
    }

    /// A press on the offline screen: Up and Down move between its rows,
    /// A does the focused one; B, X and Y as before (`failed_press`).
    pub(super) fn failed_input(&mut self, i: Input) {
        match i {
            Input::Up | Input::Down => self.move_dialog(i == Input::Down),
            Input::A => {
                let press = self.focused_act().map_or(Input::A, DialogAct::press);
                self.failed_press(press);
            }
            _ => self.failed_press(i),
        }
    }

    /// A click on row `k` of the dialog: it takes the focus and is picked,
    /// as A picks it. The second half of a double-click whose first half
    /// shut or changed the dialog (`went_on`) only moves the focus.
    pub(super) fn choose(&mut self, k: usize, went_on: bool) {
        match self.popup {
            Some(Popup::FriendOptions(who)) => self.pick_option(who, k),
            _ => self.focus_dialog(k),
        }
        if !went_on {
            self.input_inner(Input::A);
        }
    }

    /// When the dialog up now (and the invitation's notice) has come in,
    /// in seconds after it opened; none without one.
    fn dialog_intro(&self) -> Option<f64> {
        let v = self.dialog_view()?;
        let mut end = screens::intro_end(&*self.dialog_layout(v.id)?, 0);
        if matches!(self.dialog_key(), Some(DialogKey::Invite(_))) {
            if let Some(beeper) = self.dialog_layout(ScreenId::Beeper) {
                end = end.max(screens::intro_end(&beeper, 0));
            }
        }
        Some(end)
    }

    /// The dialog up now has come in (`App::menu_settled`): true without
    /// one, false before its first draw.
    pub(super) fn dialog_settled(&self) -> bool {
        let Some(end) = self.dialog_intro() else {
            return true;
        };
        self.dialog_shown == self.dialog_key() && self.menu_clock() - self.dialog_opened >= end
    }

    /// The dialog up now into `c` (`view` is the lobby layout's scale and
    /// offset in it), over everything drawn before it: the globals'
    /// overlay over the whole window, then the dialog (`screens::screen`),
    /// then for an invitation the beeper at the lower left, pulsing. Its
    /// rows and its legend's buttons go into `areas`, alone taking clicks
    /// while it is up. Its words go into `drawn` once it has come in.
    /// ADD FRIEND has only the overlay here (the lobby draws its box).
    pub(super) fn draw_dialog(
        &mut self,
        c: &mut Canvas,
        view: (f32, f32, f32),
        areas: &mut Vec<Area>,
        drawn: &mut Vec<String>,
    ) {
        self.follow_dialog();
        let add_friend = self.popup == Some(Popup::AddFriend);
        let Some(v) = self.dialog_view() else {
            if add_friend {
                self.draw_overlay(c);
            }
            return;
        };
        let Some(layout) = self.dialog_layout(v.id) else {
            return;
        };
        let clock = self.menu_clock();
        let menus = self.cfg.menus.clone();
        let (shell, fonts) = (&menus.shell, &menus.fonts);
        let art = shell.art();
        let res = Resources { art, fonts };
        let (w, h) = (c.w, c.h);
        let space = Space::new(w as u32, h as u32);
        if layout.overlay {
            self.draw_overlay(c);
        }
        let body_key = layout
            .pane(0)
            .and_then(|p| p.texts.first())
            .map(|t| t.key.clone());
        let words = |slot: &SlotText| match Some(&slot.key) == body_key.as_ref() {
            true => Words::Ours(Cow::Borrowed(v.body.as_str())),
            false => Words::Hide,
        };
        let tokens = |_: char| None;
        let pictures = |_: &h2ui::layout::BitmapWidget| Show::Keep;
        let rows: Vec<Row> = v.rows.iter().map(|(label, _)| Row::new(label.as_str())).collect();
        let focus = match self.dialog_focus.item == v.focus {
            true => self.dialog_focus,
            false => Focus::on(v.focus, self.dialog_opened),
        };
        let visible = layout
            .pane(0)
            .and_then(|p| p.lists.first())
            .map_or(rows.len(), |l| l.visible);
        let lists = [ListState {
            rows: &rows,
            focus: Some(focus),
            first: screens::first_shown(rows.len(), visible, v.focus),
        }];
        let state = ScreenState {
            pane: 0,
            opened: self.dialog_opened,
            chrome: &shell.chrome,
            words: &words,
            tokens: &tokens,
            lists: &lists,
            button_focus: None,
            disabled_buttons: &[],
            hidden_buttons: &[],
            header: Words::Ours(Cow::Borrowed(v.header.as_str())),
            legend: &v.legend,
            keyboard: self.keyboard,
            pictures: &pictures,
        };
        let (list, hits) = screens::screen(clock, &layout, &state, art, fonts, space);
        h2ui::cpu::draw_u32(&list, &mut c.px, w, h, &res);
        let came_in = clock - self.dialog_opened >= screens::intro_end(&layout, 0);
        let mut texts = list.texts;
        if let Some((_, from)) = self.invite() {
            let beeper = self.draw_beeper(c, &from, clock, &res, space);
            texts.extend(beeper);
        }
        if came_in && self.dialog_settled() {
            drawn.extend(texts);
        }
        let (s, ox, oy) = view;
        for a in hits {
            let hit = match a.hit {
                UiHit::Row(0, k) => Hit::Choice(k),
                UiHit::Legend(face) => match super::shell::input_of(face) {
                    Some(i) => Hit::Press(i),
                    None => continue,
                },
                _ => continue,
            };
            let [x0, y0, x1, y1] = a.rect;
            areas.push(Area {
                x: (x0 - ox) / s,
                y: (y0 - oy) / s,
                w: (x1 - x0) / s,
                h: (y1 - y0) / s,
                hit,
                top: true,
            });
        }
    }

    /// The globals' overlay over the whole window (`screens::overlay`).
    fn draw_overlay(&self, c: &mut Canvas) {
        let menus = &self.cfg.menus;
        let res = Resources {
            art: menus.shell.art(),
            fonts: &menus.fonts,
        };
        let space = Space::new(c.w as u32, c.h as u32);
        let over = screens::overlay(&menus.shell.chrome, space);
        h2ui::cpu::draw_u32(&over, &mut c.px, c.w, c.h, &res);
    }

    /// The invitation's notice (the beeper) at the lower left, pulsing:
    /// its icon, the tag's words on its upper line and `from` on the lower.
    /// Returns its words.
    fn draw_beeper(
        &self,
        c: &mut Canvas,
        from: &str,
        clock: f64,
        res: &Resources,
        space: Space,
    ) -> Vec<String> {
        let Some(layout) = self.dialog_layout(ScreenId::Beeper) else {
            return Vec::new();
        };
        let Some(pane) = layout.pane(0) else {
            return Vec::new();
        };
        // The lower of its two lines (both name the same string) shows the
        // gamertag.
        let lower = pane
            .texts
            .iter()
            .min_by(|a, b| a.widget.bounds[1].total_cmp(&b.widget.bounds[1]))
            .map(|t| t.widget.bounds);
        let words = |slot: &SlotText| match Some(slot.widget.bounds) == lower {
            true => Words::Ours(Cow::Owned(from.to_string())),
            false => Words::TagOr(Cow::Borrowed(BEEPER_WORDS)),
        };
        let tokens = |_: char| None;
        let pictures = |_: &h2ui::layout::BitmapWidget| Show::Keep;
        let state = ScreenState {
            pane: 0,
            opened: self.dialog_opened,
            chrome: &self.cfg.menus.shell.chrome,
            words: &words,
            tokens: &tokens,
            lists: &[],
            button_focus: None,
            disabled_buttons: &[],
            hidden_buttons: &[],
            header: Words::Hide,
            legend: &[],
            keyboard: self.keyboard,
            pictures: &pictures,
        };
        let (mut list, _) = screens::screen(clock, &layout, &state, res.art, res.fonts, space);
        pulse(&mut list, beeper_pulse(clock - self.dialog_opened));
        h2ui::cpu::draw_u32(&list, &mut c.px, c.w, c.h, res);
        list.texts
    }
}

/// Every quad of `list` faded to `alpha` of itself.
fn pulse(list: &mut DrawList, alpha: f32) {
    for q in &mut list.quads {
        for c in &mut q.color {
            c[3] *= alpha;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legends_follow_the_button_key_table() {
        let words = |k: u16| -> Vec<String> {
            legend_of(k)
                .iter()
                .map(|(f, l)| format!("{f:?} {l}"))
                .collect()
        };
        assert_eq!(words(1), ["A SELECT", "B BACK"]);
        assert_eq!(words(2), ["A SELECT", "B CANCEL"]);
        assert_eq!(words(3), ["A ENTER", "B CANCEL"]);
        assert_eq!(words(9), ["A SELECT"]);
        assert_eq!(words(0xD), ["B CANCEL"]);
        assert_eq!(words(0x16), ["A OK"]);
        assert!(words(0).is_empty() && words(7).is_empty());
    }

    #[test]
    fn the_beeper_pulses_from_full_to_a_fifth_and_back() {
        assert!((beeper_pulse(0.0) - 1.0).abs() < 1e-6);
        assert!((beeper_pulse(BEEPER_PULSE / 2.0) - PULSE_LOW).abs() < 1e-6);
        assert!((beeper_pulse(BEEPER_PULSE) - 1.0).abs() < 1e-5);
        let mut list = DrawList::default();
        list.quads.push(h2ui::paint::Quad {
            texture: h2ui::paint::Texture::White,
            rect: [0.0, 0.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [[1.0, 1.0, 1.0, 0.5]; 2],
            blend: h2ui::paint::Blend::Plain,
            wrap: [false; 2],
        });
        pulse(&mut list, 0.5);
        assert_eq!(list.quads[0].color[0][3], 0.25);
    }

    #[test]
    fn each_row_does_what_its_press_did() {
        use DialogAct as D;
        for (act, press) in [
            (D::Quit, Input::A),
            (D::SignOut, Input::Y),
            (D::Cancel, Input::B),
            (D::Leave, Input::A),
            (D::Remove, Input::A),
            (D::Unfriend, Input::A),
            (D::Option(FriendOption::Record), Input::A),
            (D::Accept, Input::A),
            (D::Decline, Input::B),
            (D::TryAgain, Input::A),
            (D::Controls, Input::X),
            (D::SignIn, Input::Y),
            (D::MainMenu, Input::B),
        ] {
            assert_eq!(act.press(), press, "{act:?}");
        }
    }
}
