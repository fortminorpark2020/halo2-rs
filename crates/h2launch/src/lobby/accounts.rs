//! The accounts screen, the playlists' last row once the server sent
//! ACCOUNTS. An admin (the server's owner) sees every account on the
//! server there: makes accounts for friends, gives them new passwords,
//! signs them out of all PCs and removes them, but never their own account
//! or another admin's (that is the server's command line). Everyone
//! changes their own password there.
//!
//! Each change is one request (`LiveClient::ask_account`, which seals any
//! password), answered with the accounts again (`Accounts::answers` says
//! which request); one is asked at a time. A password made up here is
//! shown once (NEW PASSWORD) and then forgotten; none is ever logged.
//!
//! This is a child module of `app`, to reach the lobby's state.

use super::*;
use h2live::client::AccountAsk;
use h2live::logins::MAX_PCS;
use h2live::password;
use h2live::server::{PASSWORD_CHANGED, WRONG_OLD_PASSWORD};
use h2net::live::{AccountRow, Accounts, ROW_ADMIN, ROW_ONLINE, ROW_PASSWORD};

/// How long an account request may go unanswered before the lobby says so
/// (and asks for the list again).
const ASK_WAIT: Duration = Duration::from_secs(10);
/// How long KEY CHANGED is up before A trusts the new key, and NEW
/// PASSWORD before A or B closes it: a press meant for the screen before
/// it (a second Enter, a mashed A, a B to leave) doesn't.
pub(super) const TRUST_WAIT: Duration = Duration::from_secs(1);
/// NEW ACCOUNT's rows: the gamertag typed, the password made up, CREATE.
const NEW_GAMERTAG: usize = 0;
const NEW_PASSWORD: usize = 1;
const NEW_CREATE: usize = 2;
/// CHANGE PASSWORD's fields.
const CHANGE_FIELDS: [&str; 3] = ["CURRENT PASSWORD", "NEW PASSWORD", "NEW PASSWORD AGAIN"];
/// What A says on an option for (YOU) or an admin's account.
const COMMAND_LINE: &str = "Use the server's command line for that.";
/// What A on Change my password says to an account without one.
const NO_PASSWORD: &str = "Ask the server's owner for a password first.";
const WORKING: &str = "Working...";

/// What an admin may do to an account, in the order the options popup
/// shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AccountAction {
    NewPassword,
    SignOut,
    Remove,
}

impl AccountAction {
    const ALL: [AccountAction; 3] = [
        AccountAction::NewPassword,
        AccountAction::SignOut,
        AccountAction::Remove,
    ];

    fn label(self) -> &'static str {
        match self {
            AccountAction::NewPassword => "Give a new password",
            AccountAction::SignOut => "Sign out of all PCs",
            AccountAction::Remove => "Remove account",
        }
    }

    fn title(self) -> &'static str {
        match self {
            AccountAction::NewPassword => "NEW PASSWORD",
            AccountAction::SignOut => "SIGN OUT",
            AccountAction::Remove => "REMOVE ACCOUNT",
        }
    }

    /// What the confirmation asks about `tag`'s account.
    fn question(self, tag: &str) -> String {
        match self {
            AccountAction::NewPassword => {
                format!("Give {tag} a new password? Their PCs stay signed in.")
            }
            // (Their PC is turned away at once: a game they host ends
            // with HOST_QUIT, as when a host's link dies.)
            AccountAction::SignOut => format!(
                "Sign {tag} out of all PCs? Each PC needs the password again. A game they \
                 are in ends for them, or for everyone if they host it."
            ),
            AccountAction::Remove => format!(
                "Remove {tag} for good? Their levels and friends go too. A game they are in \
                 ends for them, or for everyone if they host it."
            ),
        }
    }

    fn button(self) -> &'static str {
        match self {
            AccountAction::NewPassword => "Give new password",
            AccountAction::SignOut => "Sign out",
            AccountAction::Remove => "Remove",
        }
    }
}

/// A row of the accounts screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// Change my password: everyone's first row.
    Change,
    /// Create account: an admin's second.
    Create,
    /// An account on the server (an admin's list).
    Account(u64),
}

/// What an account request asked, to act on its answer.
pub(super) enum Asked {
    Create { gamertag: String, password: Secret },
    SetPassword { gamertag: String, password: Secret },
    SignOut,
    Remove,
    ChangePassword,
}

/// The request in flight: its number, what it asked, and when.
pub(super) struct Pending {
    pub(super) seq: u32,
    pub(super) asked: Asked,
    pub(super) at: Instant,
}

/// A password just given out, shown once: whose, whether it replaced
/// another (or came with a new account), and when it was shown.
pub(super) struct Shown {
    pub(super) gamertag: String,
    pub(super) password: Secret,
    pub(super) reset: bool,
    pub(super) at: Instant,
}

/// The accounts screen's state and its popups'.
#[derive(Default)]
pub(super) struct AccountScreen {
    /// The selected row, and its account: the selection follows the
    /// account when a new list moves it.
    pub(super) sel: usize,
    pub(super) account: Option<u64>,
    /// The open popup's row: NEW ACCOUNT's, or CHANGE PASSWORD's field.
    pub(super) row: usize,
    /// NEW ACCOUNT's gamertag, and the password made up for it.
    pub(super) gamertag: String,
    pub(super) password: Secret,
    /// CHANGE PASSWORD's fields: current, new, and new again.
    pub(super) change: [Secret; 3],
    /// What went wrong, shown in the open popup.
    pub(super) error: Option<String>,
    pub(super) pending: Option<Pending>,
    pub(super) shown: Option<Shown>,
}

impl AccountScreen {
    /// NEW ACCOUNT's gamertag has the cursor (when it's open).
    pub(super) fn typing(&self) -> bool {
        self.row == NEW_GAMERTAG
    }
}

/// "1 PC", "3 PCs".
fn pcs(n: u8) -> String {
    if n == 1 {
        "1 PC".into()
    } else {
        format!("{n} PCs")
    }
}

impl App {
    /// The accounts the server last sent, once it has.
    pub(super) fn accounts(&self) -> Option<&Accounts> {
        self.view()?.accounts.as_ref()
    }

    /// Our account is an admin's.
    pub(super) fn admin(&self) -> bool {
        self.accounts().is_some_and(|a| a.admin)
    }

    /// What the playlists' last row says.
    pub(super) fn accounts_label(&self) -> &'static str {
        if self.admin() {
            "Accounts"
        } else {
            "My account"
        }
    }

    /// The accounts screen's rows: Change my password, then for an admin
    /// Create account and every account (by gamertag, as the server sends
    /// them).
    fn account_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Change];
        if let Some(a) = self.accounts().filter(|a| a.admin) {
            rows.push(Row::Create);
            rows.extend(a.rows.iter().map(|r| Row::Account(r.account)));
        }
        rows
    }

    fn account_row(&self, id: u64) -> Option<&AccountRow> {
        self.accounts()?.rows.iter().find(|r| r.account == id)
    }

    /// An admin may change `id` from here: it isn't their own account or
    /// another admin's.
    fn may_change(&self, id: u64) -> bool {
        Some(id) != self.account()
            && self
                .account_row(id)
                .is_some_and(|r| r.flags & ROW_ADMIN == 0)
    }

    /// The account the details panel shows: the one selected, or ours on
    /// Change my password.
    fn shown_account(&self) -> Option<&AccountRow> {
        match self.account_rows().get(self.acc.sel)? {
            Row::Account(id) => self.account_row(*id),
            Row::Change => self.account_row(self.account()?),
            Row::Create => None,
        }
    }

    /// Open the accounts screen on its first row; an admin's list is asked
    /// for again.
    pub(super) fn open_accounts(&mut self) {
        self.screen = Screen::Accounts;
        self.acc.sel = 0;
        self.acc.account = None;
        self.ask_list();
    }

    /// Ask for every account again (admins only: the server drops anyone
    /// else who asks).
    fn ask_list(&mut self) {
        if !self.admin() {
            return;
        }
        if let Some(c) = &mut self.client {
            if c.ask_account(AccountAsk::List).is_some() {
                (self.cfg.log)("lobby: asked for the accounts");
            }
        }
    }

    /// Select row `k` of the accounts screen (clamped).
    pub(super) fn select_account_row(&mut self, k: usize) {
        let rows = self.account_rows();
        self.acc.sel = k.min(rows.len().saturating_sub(1));
        self.acc.account = match rows.get(self.acc.sel) {
            Some(Row::Account(id)) => Some(*id),
            _ => None,
        };
    }

    /// Select `gamertag`'s row (ignoring case), for `pick`.
    pub(super) fn pick_account(&mut self, gamertag: &str) -> bool {
        let is = |t: &str| t.eq_ignore_ascii_case(gamertag.trim());
        let at = self.account_rows().iter().position(|r| match r {
            Row::Account(id) => self.account_row(*id).is_some_and(|a| is(&a.gamertag)),
            _ => false,
        });
        at.map(|k| self.select_account_row(k)).is_some()
    }

    pub(super) fn accounts_input(&mut self, i: Input) {
        let rows = self.account_rows();
        match i {
            Input::Up => self.select_account_row(self.acc.sel.saturating_sub(1)),
            Input::Down => self.select_account_row(self.acc.sel + 1),
            Input::B => self.screen = Screen::Live,
            // One request at a time: A waits for the answer.
            Input::A if self.acc.pending.is_some() => {}
            Input::A => match rows.get(self.acc.sel) {
                Some(Row::Change) if self.accounts().is_some_and(|a| a.password) => {
                    self.acc.row = 0;
                    self.acc.change.iter_mut().for_each(Secret::clear);
                    self.acc.error = None;
                    self.popup = Some(Popup::ChangePassword);
                }
                Some(Row::Change) => self.toast(NO_PASSWORD.into()),
                Some(Row::Create) => {
                    self.acc.row = NEW_GAMERTAG;
                    self.acc.gamertag.clear();
                    self.acc.password = password::generate();
                    self.acc.error = None;
                    self.popup = Some(Popup::NewAccount);
                }
                Some(&Row::Account(id)) => {
                    self.osel = 0;
                    self.popup = Some(Popup::AccountOptions(id));
                }
                None => {}
            },
            _ => {}
        }
    }

    /// A key or button on one of the accounts popups (KEY CHANGED too).
    pub(super) fn account_popup_input(&mut self, p: Popup, i: Input) {
        match p {
            Popup::NewAccount => self.new_account_input(i),
            Popup::NewPassword => {
                // Not by a press meant for the screen before it (CREATE
                // pressed twice, a B to leave as the answer comes): it's
                // the only time the password shows.
                if matches!(i, Input::A | Input::B) && !self.password_just_shown() {
                    // Forgotten (and wiped).
                    self.acc.shown = None;
                    self.popup = None;
                }
            }
            Popup::AccountOptions(who) => self.account_options_input(who, i),
            Popup::ConfirmAccount(who, action) => self.confirm_input(who, action, i),
            Popup::ChangePassword => self.change_password_input(i),
            Popup::KeyChanged => self.key_changed_input(i),
            _ => {}
        }
    }

    /// A click on row `r` of an accounts popup: selects it, or picks it if
    /// it was selected.
    pub(super) fn account_popup_row(&mut self, r: usize) {
        match self.popup {
            Some(Popup::NewAccount) => {
                if self.acc.row == r && r != NEW_GAMERTAG {
                    self.input(Input::A);
                } else {
                    self.acc.row = r;
                }
            }
            Some(Popup::AccountOptions(_)) => {
                if self.osel == r {
                    self.input(Input::A);
                } else {
                    self.osel = r;
                }
            }
            _ => {}
        }
    }

    /// Close the accounts popup that's up, forgetting what was typed in it.
    fn close_account_popup(&mut self) {
        self.popup = None;
        self.acc.error = None;
        self.acc.gamertag.clear();
        self.acc.password.clear();
        self.acc.change.iter_mut().for_each(Secret::clear);
    }

    fn new_account_input(&mut self, i: Input) {
        let rows = 3;
        match i {
            Input::B => self.close_account_popup(),
            Input::Up => self.acc.row = (self.acc.row + rows - 1) % rows,
            Input::Down | Input::Tab => self.acc.row = (self.acc.row + 1) % rows,
            // Nothing changes while the request is out.
            _ if self.acc.pending.is_some() => {}
            Input::Backspace | Input::Char(_) if self.acc.row == NEW_GAMERTAG => {
                edit(Entry::Gamertag, &mut self.acc.gamertag, i);
                self.acc.error = None;
            }
            Input::A => match self.acc.row {
                // Enter after the gamertag: on to CREATE, the password in
                // view.
                NEW_GAMERTAG => self.acc.row = NEW_CREATE,
                NEW_PASSWORD => self.acc.password = password::generate(),
                _ => self.create_account(),
            },
            _ => {}
        }
    }

    /// Ask for the account NEW ACCOUNT describes, if its gamertag keeps
    /// the rules.
    fn create_account(&mut self) {
        let typed = settings::clean_gamertag(&self.acc.gamertag);
        let gamertag = match password::check_gamertag(&typed) {
            Ok(g) => g,
            Err(why) => {
                self.acc.error = Some(why.into());
                self.acc.row = NEW_GAMERTAG;
                return;
            }
        };
        self.acc.gamertag = gamertag.clone();
        let password = self.acc.password.clone();
        let ask = AccountAsk::Create {
            gamertag: gamertag.clone(),
            password: password.clone(),
        };
        let what = format!("create account {gamertag}");
        self.ask(ask, Asked::Create { gamertag, password }, &what);
    }

    fn account_options_input(&mut self, who: u64, i: Input) {
        let n = AccountAction::ALL.len();
        match i {
            // Up from the first goes to the last, and down from the last
            // to the first, as the friend options do.
            Input::Up => self.osel = (self.osel + n - 1) % n,
            Input::Down => self.osel = (self.osel + 1) % n,
            Input::B => self.popup = None,
            Input::A if !self.may_change(who) => self.toast(COMMAND_LINE.into()),
            Input::A => {
                let action = AccountAction::ALL[self.osel.min(n - 1)];
                self.acc.error = None;
                self.popup = Some(Popup::ConfirmAccount(who, action));
            }
            _ => {}
        }
    }

    fn confirm_input(&mut self, who: u64, action: AccountAction, i: Input) {
        match i {
            // Back to the options.
            Input::B => {
                self.acc.error = None;
                self.popup = Some(Popup::AccountOptions(who));
            }
            Input::A if self.acc.pending.is_some() => {}
            Input::A => {
                let Some(tag) = self.account_row(who).map(|r| r.gamertag.clone()) else {
                    self.popup = None;
                    return;
                };
                match action {
                    AccountAction::NewPassword => {
                        let password = password::generate();
                        let ask = AccountAsk::SetPassword {
                            account: who,
                            password: password.clone(),
                        };
                        let what = format!("give {tag} a new password");
                        let asked = Asked::SetPassword {
                            gamertag: tag,
                            password,
                        };
                        self.ask(ask, asked, &what);
                    }
                    AccountAction::SignOut => {
                        let what = format!("sign {tag} out of all PCs");
                        self.ask(AccountAsk::SignOut { account: who }, Asked::SignOut, &what);
                    }
                    AccountAction::Remove => {
                        let what = format!("remove account {tag}");
                        self.ask(AccountAsk::Remove { account: who }, Asked::Remove, &what);
                    }
                }
            }
            _ => {}
        }
    }

    fn change_password_input(&mut self, i: Input) {
        let n = CHANGE_FIELDS.len();
        match i {
            Input::B => self.close_account_popup(),
            Input::Up => self.acc.row = (self.acc.row + n - 1) % n,
            Input::Down | Input::Tab => self.acc.row = (self.acc.row + 1) % n,
            _ if self.acc.pending.is_some() => {}
            Input::Backspace | Input::Char(_) => {
                let row = self.acc.row.min(n - 1);
                edit(Entry::Password, &mut self.acc.change[row], i);
                self.acc.error = None;
            }
            Input::A => {
                // Enter moves on to the next field until all three are
                // typed.
                let empty = self.acc.change.iter().any(Secret::is_empty);
                if self.acc.row + 1 < n && empty {
                    self.acc.row += 1;
                } else {
                    self.change_password();
                }
            }
            _ => {}
        }
    }

    /// Ask to change our password, if the fields make sense: the current
    /// one typed, the new one twice the same, and keeping the rules.
    fn change_password(&mut self) {
        let [old, new, again] = &self.acc.change;
        let normal = |s: &Secret| Secret::new(password::normalize(s.as_str()));
        let problem = if old.is_empty() {
            Some((0, "Type your current password.".to_string()))
        } else if normal(new).as_str() != normal(again).as_str() {
            Some((2, "The new passwords don't match.".to_string()))
        } else {
            password::check(new.as_str())
                .err()
                .map(|why| (1, why.to_string()))
        };
        if let Some((row, why)) = problem {
            self.acc.row = row;
            self.acc.error = Some(why);
            return;
        }
        let ask = AccountAsk::ChangePassword {
            old: old.clone(),
            new: new.clone(),
        };
        self.ask(ask, Asked::ChangePassword, "change our password");
    }

    fn key_changed_input(&mut self, i: Input) {
        match i {
            // Trust it (once the popup has been up a moment): pin it, and
            // sign in again.
            Input::A => {
                if self.now.saturating_duration_since(self.key_changed_at) < TRUST_WAIT {
                    return;
                }
                self.popup = None;
                let Some((server, _, now)) = self.key_changed.take() else {
                    return;
                };
                self.pins.set(&server, now);
                let code = h2live::seal::short_code(&now);
                match self.pins.save(&self.cfg.folder) {
                    Ok(()) => self.log(&format!("lobby: trusting {server}'s new key {code}")),
                    Err(e) => self.log(&format!(
                        "lobby: {server}'s new key {code} couldn't be saved in {}: {e}",
                        crate::pins::path(&self.cfg.folder).display()
                    )),
                }
                self.connect();
            }
            Input::B => {
                self.log("lobby: the server's new key wasn't trusted");
                self.key_changed = None;
                self.password.clear();
                self.with_password = false;
                self.popup = None;
                self.screen = Screen::Main;
            }
            _ => {}
        }
    }

    /// Send account request `ask` (`asked`, to act on the answer; `what`,
    /// for the log), unless one is in flight already.
    fn ask(&mut self, ask: AccountAsk, asked: Asked, what: &str) {
        if self.acc.pending.is_some() {
            return;
        }
        self.acc.error = None;
        match self.client.as_mut().and_then(|c| c.ask_account(ask)) {
            Some(seq) => {
                self.log(&format!("lobby: asked to {what}"));
                self.acc.pending = Some(Pending {
                    seq,
                    asked,
                    at: self.now,
                });
            }
            None => self.account_error("The server can't be asked now. Sign in again.".into()),
        }
    }

    /// Show what went wrong with an account request: in the popup it came
    /// from, or as a notice.
    fn account_error(&mut self, text: String) {
        self.log(&format!("lobby: account request: {text}"));
        match self.popup {
            Some(Popup::NewAccount | Popup::ChangePassword | Popup::ConfirmAccount(..)) => {
                self.acc.error = Some(text);
            }
            _ => self.toast(text),
        }
    }

    /// New accounts came (`View::accounts`, the newest): if that message
    /// answered the request in flight (`answers`, with `ok` and `text`),
    /// act on the answer. (The screen draws from the view, and the
    /// selection follows its account at the end of the tick.)
    pub(super) fn on_accounts(&mut self, answers: u32, ok: bool, text: String) {
        let answered = |p: &Pending| answers != 0 && p.seq == answers;
        if !self.acc.pending.as_ref().is_some_and(answered) {
            return;
        }
        let Some(p) = self.acc.pending.take() else {
            return;
        };
        if !ok {
            // The field at fault gets the cursor, as with the lobby's own
            // checks: a wrong current password is rubbed out, and a
            // gamertag the server won't take is there to change.
            match p.asked {
                Asked::ChangePassword
                    if self.popup == Some(Popup::ChangePassword)
                        && (text == WRONG_OLD_PASSWORD || text == TOO_MANY_TRIES) =>
                {
                    self.acc.change[0].clear();
                    self.acc.row = 0;
                }
                // Changed, but the server couldn't save it: the new password
                // works now, so it's the current one if A is pressed again
                // (the old one would be refused, and counted as wrong).
                Asked::ChangePassword
                    if self.popup == Some(Popup::ChangePassword)
                        && text.starts_with(PASSWORD_CHANGED) =>
                {
                    self.acc.change[0] = self.acc.change[1].clone();
                }
                Asked::Create { .. }
                    if self.popup == Some(Popup::NewAccount)
                        && (text == GAMERTAG_TAKEN || text == password::GAMERTAG_RULES) =>
                {
                    self.acc.row = NEW_GAMERTAG;
                }
                _ => {}
            }
            return self.account_error(text);
        }
        match p.asked {
            Asked::Create { gamertag, password } => {
                self.acc.gamertag.clear();
                self.acc.password.clear();
                // The new account is selected in the list.
                let id = self.accounts().and_then(|a| {
                    let row = a.rows.iter().find(|r| r.gamertag == gamertag);
                    row.map(|r| r.account)
                });
                if id.is_some() {
                    self.acc.account = id;
                }
                self.log(&format!("lobby: account {gamertag} created"));
                self.show_password(gamertag, password, false);
            }
            Asked::SetPassword { gamertag, password } => {
                self.log(&format!("lobby: {gamertag} has a new password"));
                self.show_password(gamertag, password, true);
            }
            Asked::SignOut | Asked::Remove => {
                if matches!(self.popup, Some(Popup::ConfirmAccount(..))) {
                    self.popup = None;
                }
                self.toast(text);
            }
            Asked::ChangePassword => {
                if self.popup == Some(Popup::ChangePassword) {
                    self.close_account_popup();
                }
                self.toast("Password changed.".into());
            }
        }
    }

    /// Show the password just given to `gamertag`, once.
    fn show_password(&mut self, gamertag: String, password: Secret, reset: bool) {
        self.acc.error = None;
        self.acc.shown = Some(Shown {
            gamertag,
            password,
            reset,
            at: self.now,
        });
        self.popup = Some(Popup::NewPassword);
    }

    /// NEW PASSWORD has been up less than `TRUST_WAIT`: A and B do nothing
    /// yet.
    fn password_just_shown(&self) -> bool {
        let shown = self.acc.shown.as_ref();
        shown.is_some_and(|s| self.now.saturating_duration_since(s.at) < TRUST_WAIT)
    }

    /// Each tick: a request unanswered for too long is given up on (and
    /// the list asked for again), the selection follows its account, and a
    /// popup about an account that's gone (or an admin's popup, for an
    /// account that's no longer one) closes.
    pub(super) fn tick_accounts(&mut self) {
        if self.client.is_none() {
            self.acc.pending = None;
        }
        let late = |p: &Pending| self.now.saturating_duration_since(p.at) >= ASK_WAIT;
        if self.acc.pending.as_ref().is_some_and(late) {
            self.acc.pending = None;
            self.toast("No answer from the server. Check the list.".into());
            self.ask_list();
        }
        let keys: Vec<Option<u64>> = self
            .account_rows()
            .iter()
            .map(|r| match r {
                Row::Account(id) => Some(*id),
                _ => None,
            })
            .collect();
        if let Some(at) = self
            .acc
            .account
            .and_then(|a| keys.iter().position(|&k| k == Some(a)))
        {
            self.acc.sel = at;
        }
        self.acc.sel = self.acc.sel.min(keys.len().saturating_sub(1));
        self.acc.account = keys.get(self.acc.sel).copied().flatten();
        if self.acc.pending.is_some() {
            return;
        }
        let gone = match self.popup {
            Some(Popup::AccountOptions(id) | Popup::ConfirmAccount(id, _)) => {
                self.account_row(id).is_none()
            }
            Some(Popup::NewAccount) => !self.admin(),
            _ => false,
        };
        if gone {
            self.close_account_popup();
        }
    }

    // ---------------------------------------------------------- drawing

    /// The accounts screen: its rows on the left, and the account selected
    /// on the right.
    pub(super) fn draw_accounts(&self, p: &mut Pen) {
        let Some(a) = self.accounts() else {
            return;
        };
        p.panel(60.0, 110.0, 720.0, 530.0);
        if a.admin {
            p.text(84.0, 142.0, 16.0, HEAD, Align::Left, "GAMERTAG");
            p.text(380.0, 142.0, 16.0, HEAD, Align::Center, "PCS");
            let total = if a.total == 1 {
                "1 account".to_string()
            } else {
                format!("{} accounts", a.total)
            };
            p.text(756.0, 142.0, 16.0, DIM, Align::Right, &total);
        }
        let rows = self.account_rows();
        let first = first_row(self.acc.sel, rows.len(), LIST_ROWS);
        for (k, row) in rows.iter().enumerate().skip(first).take(LIST_ROWS) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.acc.sel;
            p.row(72.0, y, 696.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            match *row {
                Row::Change => {
                    let col = if a.password { col } else { DIM };
                    p.text(90.0, y + 32.0, 24.0, col, Align::Left, "Change my password");
                }
                Row::Create => {
                    p.text(90.0, y + 32.0, 24.0, col, Align::Left, "Create account");
                }
                Row::Account(id) => {
                    let Some(r) = self.account_row(id) else {
                        continue;
                    };
                    let name = p.fit(24.0, 240.0, &r.gamertag);
                    p.text(90.0, y + 32.0, 24.0, col, Align::Left, &name);
                    p.text(380.0, y + 32.0, 20.0, DIM, Align::Center, &pcs(r.pcs));
                    let mut x = 432.0;
                    for (tag, tcol) in self.account_tags(r) {
                        x += p.text(x, y + 31.0, 15.0, tcol, Align::Left, tag) + 12.0;
                    }
                }
            }
        }
        let foot = if self.acc.pending.is_some() {
            WORKING.to_string()
        } else if a.admin {
            format!(
                "Each account signs in on up to {MAX_PCS} PCs, typing its password once on each."
            )
        } else {
            "Your PCs stay signed in when you change your password.".to_string()
        };
        let foot = p.fit(17.0, 680.0, &foot);
        p.text(84.0, 622.0, 17.0, DIM, Align::Left, &foot);
        self.draw_account_details(p, rows.get(self.acc.sel).copied());
        let mut hints = Vec::new();
        match rows.get(self.acc.sel) {
            Some(Row::Change) if a.password => hints.push((Input::A, "Change password")),
            Some(Row::Create) => hints.push((Input::A, "Create account")),
            Some(Row::Account(_)) => hints.push((Input::A, "Options")),
            _ => {}
        }
        hints.push((Input::B, "Back"));
        p.hints(&hints);
    }

    /// An account row's tags, each with its colour.
    fn account_tags(&self, r: &AccountRow) -> Vec<(&'static str, Color)> {
        let mut tags = Vec::new();
        if r.flags & ROW_ADMIN != 0 {
            tags.push(("ADMIN", GOLD));
        }
        if r.flags & ROW_PASSWORD == 0 {
            tags.push(("NO PASSWORD", WARN));
        }
        if r.flags & ROW_ONLINE != 0 {
            tags.push(("ONLINE", GOOD));
        }
        if Some(r.account) == self.account() {
            tags.push(("(YOU)", DIM));
        }
        tags
    }

    /// The panel on the right: the account selected (ours on Change my
    /// password), or what Create account does.
    fn draw_account_details(&self, p: &mut Pen, row: Option<Row>) {
        p.panel(810.0, 110.0, 410.0, 530.0);
        if row == Some(Row::Create) {
            p.text(834.0, 170.0, 30.0, WHITE, Align::Left, "New account");
            let help = "An account for a friend: you pick its gamertag, and the launcher makes \
                        up a password for you to give them. They type both once on each PC.";
            for (k, line) in p.wrap(18.0, 362.0, help).iter().enumerate() {
                p.text(
                    834.0,
                    214.0 + k as f32 * 26.0,
                    18.0,
                    TEXT,
                    Align::Left,
                    line,
                );
            }
            return;
        }
        let Some(r) = self.shown_account() else {
            return;
        };
        let name = p.fit(30.0, 362.0, &r.gamertag);
        p.text(834.0, 170.0, 30.0, WHITE, Align::Left, &name);
        let yes = |b: bool| if b { "Yes" } else { "No" };
        let password = r.flags & ROW_PASSWORD != 0;
        let lines = [
            ("PCs", format!("{} of {MAX_PCS}", r.pcs), TEXT),
            (
                "Password",
                yes(password).to_string(),
                if password { TEXT } else { WARN },
            ),
            ("Online", yes(r.flags & ROW_ONLINE != 0).to_string(), TEXT),
        ];
        for (k, (label, value, col)) in lines.iter().enumerate() {
            let y = 220.0 + k as f32 * 36.0;
            p.text(834.0, y, 20.0, DIM, Align::Left, label);
            p.text(1196.0, y, 20.0, *col, Align::Right, value);
        }
        let y = 220.0 + 3.0 * 36.0;
        p.text(834.0, y, 20.0, DIM, Align::Left, "Level");
        p.level(
            1196.0,
            y,
            20.0,
            TEXT,
            Align::Right,
            r.best.max(1),
            Size::Small,
        );
        let note = if Some(r.account) == self.account() {
            if password {
                "This is you."
            } else {
                "This is you. Ask the server's owner for a password to change it or to sign in \
                 on another PC."
            }
        } else if r.flags & ROW_ADMIN != 0 {
            "An admin: only the server's command line changes this account."
        } else if !password {
            "No password yet: give one so they can sign in on a new PC."
        } else {
            ""
        };
        for (k, line) in p.wrap(17.0, 362.0, note).iter().enumerate() {
            p.text(834.0, 400.0 + k as f32 * 24.0, 17.0, DIM, Align::Left, line);
        }
    }

    pub(super) fn draw_account_popup(&self, p: &mut Pen, popup: Popup) {
        match popup {
            Popup::NewAccount => self.draw_new_account(p),
            Popup::NewPassword => self.draw_new_password(p),
            Popup::AccountOptions(who) => self.draw_account_options(p, who),
            Popup::ConfirmAccount(who, action) => {
                let tag = self
                    .account_row(who)
                    .map_or("this player".to_string(), |r| r.gamertag.clone());
                let hints = [(Input::A, action.button()), (Input::B, "Cancel")];
                // What went wrong takes the question's place, so all of it
                // shows (whether it was done, and what to do next).
                if let Some(e) = &self.acc.error {
                    p.popup(action.title(), "", &hints);
                    for (k, line) in p.wrap_at_most(18.0, 560.0, e, 4).iter().enumerate() {
                        let y = 340.0 + k as f32 * 24.0;
                        p.text(640.0, y, 18.0, ERROR, Align::Center, line);
                    }
                } else {
                    p.popup(action.title(), &action.question(&tag), &hints);
                    self.draw_popup_status(p, 436.0, 1, "");
                }
            }
            Popup::ChangePassword => self.draw_change_password(p),
            Popup::KeyChanged => self.draw_key_changed(p),
            _ => {}
        }
    }

    /// The open popup's status, from `y` down: what went wrong (on up to
    /// `lines` lines, so the rule it broke or what to do next isn't cut
    /// off), else that the request is out, else `help`.
    fn draw_popup_status(&self, p: &mut Pen, y: f32, lines: usize, help: &str) {
        let (text, col) = match (&self.acc.error, &self.acc.pending) {
            (Some(e), _) => (e.as_str(), ERROR),
            (None, Some(_)) => (WORKING, DIM),
            (None, None) => (help, DIM),
        };
        for (k, line) in p.wrap_at_most(18.0, 580.0, text, lines).iter().enumerate() {
            let y = y + k as f32 * 22.0;
            p.text(640.0, y, 18.0, col, Align::Center, line);
        }
    }

    fn draw_new_account(&self, p: &mut Pen) {
        p.popup_box(320.0, 150.0, 640.0, 390.0);
        p.text(640.0, 194.0, 30.0, WHITE, Align::Center, "NEW ACCOUNT");
        let fields = [("GAMERTAG", 228.0), ("PASSWORD", 306.0)];
        for (k, (label, y)) in fields.into_iter().enumerate() {
            let on = self.acc.row == k;
            p.text(380.0, y, 15.0, HEAD, Align::Left, label);
            p.fill(
                380.0,
                y + 8.0,
                520.0,
                44.0,
                Color::rgb(0x06_1430).alpha(220),
            );
            let edge = if on { SEL_EDGE } else { EDGE };
            p.outline(380.0, y + 8.0, 520.0, 44.0, 2.0, edge);
            p.area(380.0, y + 8.0, 520.0, 44.0, Hit::Row(k));
            if k == NEW_GAMERTAG {
                let shown = p.fit(24.0, 480.0, &self.acc.gamertag);
                let wd = p.text(396.0, y + 40.0, 24.0, WHITE, Align::Left, &shown);
                if on {
                    p.fill(398.0 + wd, y + 16.0, 2.0, 28.0, SEL_EDGE);
                }
            } else {
                // Shown on purpose: the admin gives it out.
                p.text(
                    396.0,
                    y + 40.0,
                    24.0,
                    WHITE,
                    Align::Left,
                    self.acc.password.as_str(),
                );
                if on {
                    p.text(888.0, y + 38.0, 15.0, DIM, Align::Right, "A makes another");
                }
            }
        }
        let on = self.acc.row == NEW_CREATE;
        p.row(380.0, 376.0, 520.0, 44.0, on, Hit::Row(NEW_CREATE));
        if !on {
            p.outline(380.0, 376.0, 520.0, 44.0, 2.0, EDGE);
        }
        let create = if self.acc.pending.is_some() {
            "WORKING..."
        } else {
            "CREATE"
        };
        let col = if on { WHITE } else { TEXT };
        p.text(640.0, 406.0, 22.0, col, Align::Center, create);
        let help = match self.acc.row {
            NEW_GAMERTAG => "Type the gamertag on the keyboard: letters, numbers and spaces.",
            NEW_PASSWORD => "Give them this password. A makes another.",
            _ => "They type the gamertag and password once on each PC.",
        };
        self.draw_popup_status(p, 448.0, 3, help);
        let a = match self.acc.row {
            NEW_GAMERTAG => "Next",
            NEW_PASSWORD => "Another password",
            _ => "Create",
        };
        let hints = [(Input::A, a), (Input::B, "Cancel")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 520.0);
    }

    fn draw_new_password(&self, p: &mut Pen) {
        p.popup_box(320.0, 200.0, 640.0, 320.0);
        let Some(s) = &self.acc.shown else {
            return;
        };
        let (title, line) = if s.reset {
            ("NEW PASSWORD", format!("{}'s new password is", s.gamertag))
        } else {
            (
                "ACCOUNT CREATED",
                format!("{} can sign in now with the password", s.gamertag),
            )
        };
        p.text(640.0, 250.0, 30.0, WHITE, Align::Center, title);
        let line = p.fit(22.0, 580.0, &line);
        p.text(640.0, 298.0, 22.0, TEXT, Align::Center, &line);
        // Shown on purpose, once.
        p.text(
            640.0,
            352.0,
            36.0,
            WHITE,
            Align::Center,
            s.password.as_str(),
        );
        let give = "Give it to them; it won't be shown again.";
        p.text(640.0, 398.0, 20.0, TEXT, Align::Center, give);
        let after = if s.reset {
            "Their PCs stay signed in."
        } else {
            "They type it once on each PC."
        };
        p.text(640.0, 428.0, 17.0, DIM, Align::Center, after);
        let hints = [(Input::A, "Done")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 492.0);
    }

    fn draw_account_options(&self, p: &mut Pen, who: u64) {
        p.popup_box(320.0, 200.0, 640.0, 320.0);
        let name = self.account_row(who).map_or("", |r| r.gamertag.as_str());
        p.text(640.0, 250.0, 30.0, WHITE, Align::Center, name);
        let allowed = self.may_change(who);
        for (k, action) in AccountAction::ALL.iter().enumerate() {
            let y = 280.0 + k as f32 * 44.0;
            let on = k == self.osel;
            p.row(360.0, y, 560.0, 40.0, on, Hit::Row(k));
            let col = match (allowed, on) {
                (false, _) => DIM,
                (true, true) => WHITE,
                (true, false) => TEXT,
            };
            p.text(640.0, y + 28.0, 22.0, col, Align::Center, action.label());
        }
        if !allowed {
            let why = if Some(who) == self.account() {
                "Your own account: use the server's command line."
            } else {
                "An admin's account: use the server's command line."
            };
            p.text(640.0, 444.0, 17.0, DIM, Align::Center, why);
        }
        let hints = [(Input::A, "Select"), (Input::B, "Back")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 492.0);
    }

    fn draw_change_password(&self, p: &mut Pen) {
        p.popup_box(320.0, 130.0, 640.0, 410.0);
        p.text(640.0, 174.0, 30.0, WHITE, Align::Center, "CHANGE PASSWORD");
        for (k, label) in CHANGE_FIELDS.iter().enumerate() {
            let y = 206.0 + k as f32 * 80.0;
            let on = self.acc.row == k;
            p.text(380.0, y, 15.0, HEAD, Align::Left, label);
            p.fill(
                380.0,
                y + 8.0,
                520.0,
                42.0,
                Color::rgb(0x06_1430).alpha(220),
            );
            let edge = if on { SEL_EDGE } else { EDGE };
            p.outline(380.0, y + 8.0, 520.0, 42.0, 2.0, edge);
            // Masked before it's drawn.
            let stars = masked(&self.acc.change[k]);
            let shown = p.fit(22.0, 480.0, &stars);
            let wd = p.text(396.0, y + 38.0, 22.0, WHITE, Align::Left, &shown);
            if on {
                p.fill(398.0 + wd, y + 15.0, 2.0, 26.0, SEL_EDGE);
            }
            p.area(380.0, y + 8.0, 520.0, 42.0, Hit::Field(k));
        }
        let help = "Type on the keyboard. Your PCs stay signed in.";
        self.draw_popup_status(p, 446.0, 3, help);
        let hints = [(Input::A, "Change"), (Input::B, "Cancel")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 520.0);
    }

    fn draw_key_changed(&self, p: &mut Pen) {
        p.popup_box(320.0, 170.0, 640.0, 350.0);
        p.text(
            640.0,
            216.0,
            30.0,
            WHITE,
            Align::Center,
            "SERVER KEY CHANGED",
        );
        if let Some((server, was, now)) = &self.key_changed {
            let body = format!(
                "The server at {server} answered with key {} instead of {}. Someone may be \
                 pretending to be it. Only go on if its owner told you it changed.",
                h2live::seal::short_code(now),
                h2live::seal::short_code(was)
            );
            for (k, line) in p.wrap(20.0, 560.0, &body).iter().take(7).enumerate() {
                p.text(
                    640.0,
                    258.0 + k as f32 * 28.0,
                    20.0,
                    TEXT,
                    Align::Center,
                    line,
                );
            }
        }
        let hints = [(Input::A, "Trust the new key"), (Input::B, "Cancel")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 494.0);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, live_app, live_app_on, scratch, to_online, view};
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use h2live::password::Cost;
    use h2live::seal;
    use h2live::server::{Actor, Command, Route, Server, SignUp};
    use h2net::live::{
        Login, LoginClient, ToPc, Welcome, ACCOUNT_REMOVED, NO_ACCOUNT, SERVER_BUSY,
    };
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::Path;
    use std::sync::Mutex;

    /// Passwords for the tests, made up here.
    const PASSWORD: &str = "K7MP-4QR9-XW2T";
    const ROOT_PASSWORD: &str = "ROOT-PASS-7731";
    /// Where the test lobbies sign in. Nothing listens there: each dial
    /// is handed its connection by the test.
    const SERVER: &str = "127.0.0.1:9";

    /// A lobby's log lines.
    type Said = Arc<Mutex<Vec<String>>>;

    /// A lobby on `folder` whose log is kept.
    fn logged(folder: &Path) -> (App, Said) {
        let said: Said = Arc::default();
        let kept = said.clone();
        let log: Log = Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string()));
        let mut a = app(folder);
        a.cfg.log = log;
        (a, said)
    }

    /// `logged`, with `lobby.txt` saying to sign in at SERVER as
    /// `gamertag` (none if empty).
    fn lobby(folder: &Path, gamertag: &str) -> (App, Said) {
        Settings {
            server: SERVER.into(),
            gamertag: gamertag.into(),
            ..Settings::default()
        }
        .save(&folder.join("lobby.txt"))
        .unwrap();
        logged(folder)
    }

    fn log_has(said: &Said, text: &str) -> bool {
        said.lock().unwrap().iter().any(|l| l.contains(text))
    }

    fn type_in(a: &mut App, text: &str) {
        for c in text.chars() {
            a.input(Input::Char(c));
        }
    }

    /// Type `gamertag` and the password `typed` on the sign-in form (over
    /// what was there), and press A.
    fn fill_form(a: &mut App, gamertag: &str, typed: &str) {
        assert_eq!(a.label(), "signin");
        a.field = GAMERTAG_FIELD;
        for _ in 0..=GAMERTAG_LEN {
            a.input(Input::Backspace);
        }
        type_in(a, gamertag);
        a.input(Input::Tab);
        assert_eq!(a.field, PASSWORD_FIELD);
        for _ in 0..2 * MAX_PASSWORD {
            a.input(Input::Backspace);
        }
        type_in(a, typed);
        a.input(Input::A);
    }

    /// A dial that got through, on `conn`.
    fn through(conn: Connection) -> Receiver<Result<Connection, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(conn)).unwrap();
        rx
    }

    /// A real h2live server in this process, on a clock of its own: only
    /// its owner makes accounts, and its password hashes are cheap.
    struct Live {
        dir: PathBuf,
        server: Server,
        now: f64,
        dials: u8,
    }

    impl Live {
        fn new(name: &str) -> Live {
            let dir = scratch(&format!("{name}-server"));
            let mut server = Server::open(&dir, Some("test")).unwrap();
            server.set_sign_up(SignUp::Closed);
            server.set_password_cost(Cost::Cheap);
            Live {
                dir,
                server,
                now: 0.0,
                dials: 0,
            }
        }

        /// An account made on the server's command line: its id.
        fn add(&mut self, gamertag: &str, typed: &str, admin: bool) -> u64 {
            let phc = password::hash(typed, Cost::Cheap).unwrap();
            let add = Command::Add {
                gamertag: gamertag.into(),
                phc,
                admin,
            };
            self.server.command(Actor::Console, add, self.now).unwrap();
            self.server.find(gamertag).unwrap()
        }

        /// A lobby dialling gets through: the server takes one end of a
        /// pair (from an address of its own each time), the lobby the
        /// other.
        fn answer(&mut self, a: &mut App) {
            if a.dialing.is_none() {
                return;
            }
            let (server_end, pc_end) = Connection::pair();
            self.dials += 1;
            let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, self.dials));
            self.server.accept(server_end, Route::Live, ip, self.now);
            a.dialing = Some((through(pc_end), Instant::now()));
        }

        fn step(&mut self, apps: &mut [&mut App]) {
            for a in apps.iter_mut() {
                self.answer(a);
                a.tick(Instant::now());
            }
            self.now += 0.05;
            self.server.poll(self.now);
        }

        /// Run the lobbies and the server until `done`, for 20 s of the
        /// server's clock at most.
        fn until(&mut self, apps: &mut [&mut App], done: impl Fn(&Server, &[&mut App]) -> bool) {
            for _ in 0..400 {
                if done(&self.server, apps) {
                    return;
                }
                self.step(apps);
            }
            let labels: Vec<String> = apps.iter().map(|a| a.label()).collect();
            panic!("still waiting, on {labels:?}");
        }

        /// Run `a` until it's through signing in.
        fn sign_in(&mut self, a: &mut App) {
            assert_eq!(a.label(), "connecting");
            self.until(&mut [&mut *a], |_, a| a[0].label() != "connecting");
        }
    }

    impl Drop for Live {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A PC of its own (a new folder, its log kept) signs in on `live`
    /// through the form, and is on the playlists once the accounts come.
    fn new_pc(live: &mut Live, name: &str, gamertag: &str, typed: &str) -> (App, Said, PathBuf) {
        let dir = scratch(name);
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        fill_form(&mut a, gamertag, typed);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live", "{:?}", a.form_error);
        live.until(&mut [&mut a], |_, a| a[0].accounts().is_some());
        (a, said, dir)
    }

    /// From the playlists: down to the accounts row, and A.
    fn open_accounts_row(a: &mut App) {
        let row = a.accounts_row().unwrap();
        for _ in 0..row + 3 {
            a.input(Input::Down);
        }
        assert_eq!(a.sel, row);
        a.input(Input::A);
        assert_eq!(a.label(), "accounts");
    }

    /// Draw `a`, if there is a font to draw with: false if there isn't.
    fn drew(a: &mut App) -> bool {
        let Ok(mut text) = Text::system() else {
            return false;
        };
        let mut c = Canvas::new(1280, 720);
        a.draw(&mut c, &mut text);
        true
    }

    fn drawn(a: &App, text: &str) -> bool {
        a.drawn().iter().any(|d| d.contains(text))
    }

    fn toasted(a: &App, text: &str) -> bool {
        a.toasts.iter().any(|t| t.0 == text)
    }

    /// A server the test plays itself, to see every byte a lobby sends:
    /// its identity key, the key it seals to, its challenge, and the nonce
    /// in the lobby's LOGIN (once `heard`).
    struct ByHand {
        conn: Connection,
        identity: SigningKey,
        secret: [u8; 32],
        nonce: [u8; 32],
        login_nonce: Option<[u8; 32]>,
    }

    impl ByHand {
        /// Take the lobby's dial, as the server with identity key number
        /// `n` (the lobby's LOGIN comes at once).
        fn answer(a: &mut App, n: u8) -> ByHand {
            assert!(a.dialing.is_some());
            let (conn, pc_end) = Connection::pair();
            a.dialing = Some((through(pc_end), Instant::now()));
            a.tick(Instant::now());
            ByHand {
                conn,
                identity: SigningKey::from_bytes(&[n; 32]),
                secret: [7; 32],
                nonce: [9; 32],
                login_nonce: None,
            }
        }

        fn key(&self) -> [u8; 32] {
            self.identity.verifying_key().to_bytes()
        }

        /// SEAL_KEY, signed with the identity key over the LOGIN `heard`,
        /// then the challenge.
        fn challenge(&mut self) {
            let key = seal::public(&self.secret);
            let theirs = self.login_nonce.expect("the LOGIN wasn't heard");
            let proof = seal::key_proof(&theirs, &self.nonce, &key);
            let sig = self.identity.sign(&proof);
            let server = self.key();
            let sig = sig.to_bytes();
            ToPc::SealKey { key, server, sig }.send(&mut self.conn);
            let motd = String::new();
            let nonce = self.nonce;
            ToPc::Challenge { nonce, motd }.send(&mut self.conn);
            self.conn.flush().unwrap();
        }

        fn welcome(&mut self, gamertag: &str) {
            ToPc::Welcome(Welcome {
                account: 1,
                gamertag: gamertag.into(),
                card: String::new(),
                best: 1,
                levels: Vec::new(),
            })
            .send(&mut self.conn);
            self.conn.flush().unwrap();
        }

        /// What the lobby sent since the last look, read and as it came
        /// (pings left out), whether or not it has hung up since.
        fn heard(&mut self) -> Vec<(ToServer, Vec<u8>)> {
            let got = self.conn.receive().unwrap_or_default();
            let heard: Vec<_> = got
                .into_iter()
                .map(|(k, b)| (ToServer::read(k, &b).unwrap(), b))
                .filter(|(m, _)| !matches!(m, ToServer::Ping(_) | ToServer::Pong(_)))
                .collect();
            for (message, _) in &heard {
                if let ToServer::Login(Login {
                    client: LoginClient::Launcher { nonce, .. },
                    ..
                }) = message
                {
                    self.login_nonce = Some(*nonce);
                }
            }
            heard
        }
    }

    /// `bytes` hold `text`.
    fn holds(bytes: &[u8], text: &str) -> bool {
        bytes.windows(text.len()).any(|w| w == text.as_bytes())
    }

    #[test]
    fn the_password_is_never_drawn_or_logged() {
        let dir = scratch("never-shown");
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        type_in(&mut a, "alpha");
        a.input(Input::Tab);
        // In lower case with spaces, as a player might type it.
        let typed = "k7mp 4qr9 xw2t";
        type_in(&mut a, typed);
        assert_eq!(a.password.as_str(), typed);
        let forms = [
            typed.to_string(),
            typed.to_uppercase(),
            password::normalize(typed),
        ];
        // Drawn masked: a star for each character, and nothing of it.
        if drew(&mut a) {
            let stars = "*".repeat(typed.len());
            assert!(a.drawn().contains(&stars), "{:?}", a.drawn());
            for f in &forms {
                assert!(!drawn(&a, f) && !drawn(&a, "k7mp") && !drawn(&a, "K7MP"));
            }
        }
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        let mut server = ByHand::answer(&mut a, 3);
        let heard = server.heard();
        let [(ToServer::Login(login), _)] = &heard[..] else {
            panic!("{heard:?}");
        };
        let me = login.key;
        server.challenge();
        a.tick(Instant::now());
        let heard = server.heard();
        let [(prove, raw)] = &heard[..] else {
            panic!("{heard:?}");
        };
        let ToServer::ProvePassword {
            password: sealed,
            signature,
            computer,
        } = prove
        else {
            panic!("{prove:?}");
        };
        // Signed for this LOGIN's gamertag, this PC's name and this box.
        let proof = h2net::live::password_proof(&server.nonce, &me, "ALPHA", computer, sealed);
        let pc = ed25519_dalek::VerifyingKey::from_bytes(&me).unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(signature);
        assert!(pc.verify_strict(&proof, &signature).is_ok());
        // Not in the bytes sent, nor in what the log would say of them.
        for f in &forms {
            assert!(!holds(raw, f), "{f}");
            assert!(!live::describe(prove).contains(f.as_str()));
            assert!(!format!("{prove:?}").contains(f.as_str()));
        }
        // Sealed for the server, which alone opens it (as typed, but for
        // case, spaces and dashes).
        let aad = seal::aad(seal::SIGN_IN, &server.nonce, &me, b"ALPHA");
        let plain = seal::open(&server.secret, &aad, sealed).unwrap();
        let opened = seal::unpad(&plain).unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].as_str(), password::normalize(typed));
        // Welcomed: it's forgotten, and the server's key pinned.
        server.welcome("ALPHA");
        a.tick(Instant::now());
        assert_eq!(a.label(), "live");
        assert!(a.password.is_empty());
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(server.key()));
        // No log line and no file the lobby keeps has it.
        assert!(log_has(&said, "with a password"));
        let log = said.lock().unwrap().join("\n");
        for f in &forms {
            assert!(!log.contains(f.as_str()), "{log}");
        }
        for file in std::fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            for f in &forms {
                assert!(!holds(&bytes, f), "{}", path.display());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_pc_signs_in_with_a_password_against_a_real_server() {
        let mut live = Live::new("new-pc");
        let alpha = live.add("ALPHA", PASSWORD, false);
        live.add("BRAVO", ROOT_PASSWORD, false);
        let dir = scratch("new-pc");
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        // Typed in lower case: gamertags and passwords are matched
        // either way.
        fill_form(&mut a, "alpha", &PASSWORD.to_lowercase());
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live", "{:?}", a.form_error);
        assert_eq!(a.account(), Some(alpha));
        assert!(a.password.is_empty() && !a.with_password);
        // The gamertag (as the server has it) and the server's key are
        // kept for next time; the PC is one of the account's.
        assert_eq!(Settings::load(&dir.join("lobby.txt")).gamertag, "ALPHA");
        let key = live.server.identity_key();
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(key));
        assert!(log_has(&said, "lobby: pinned 127.0.0.1:9's key "));
        assert_eq!(live.server.devices_of(alpha).len(), 1);
        // A player's own account comes after the welcome.
        live.until(&mut [&mut a], |_, a| a[0].accounts().is_some());
        let accounts = a.accounts().unwrap();
        assert!(!accounts.admin && accounts.password);
        assert_eq!(accounts.rows.len(), 1);
        drop(a);
        live.until(&mut [], |s, _| s.players_online() == 0);
        // The next time, on the same folder, ONLINE signs in with no
        // password: the PC's key is enough.
        let (mut a, said) = logged(&dir);
        to_online(&mut a);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live");
        assert_eq!(a.account(), Some(alpha));
        assert!(!log_has(&said, "with a password"));
        assert_eq!(live.server.devices_of(alpha).len(), 1);
        drop(a);
        live.until(&mut [], |s, _| s.players_online() == 0);
        // Another gamertag typed with no password: still this PC's
        // account, and the lobby says how to use the other.
        let (mut a, _said) = logged(&dir);
        a.input(Input::A);
        a.screen = Screen::SignIn;
        fill_form(&mut a, "bravo", "");
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live");
        assert_eq!(a.settings.gamertag, "ALPHA");
        assert_eq!(a.fields[GAMERTAG_FIELD], "ALPHA");
        assert!(toasted(
            &a,
            "This PC is signed in as ALPHA. To use another gamertag here, type its password too."
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_password_typed_in_the_gamertag_box_is_never_logged_or_saved() {
        // The GAMERTAG box keeps all of a made-up password but its dashes,
        // which don't count: until the server welcomes a gamertag, it's
        // neither logged (with a password) nor saved in lobby.txt.
        let mut live = Live::new("swapped");
        let kat = live.add("KAT", PASSWORD, false);
        let dir = scratch("swapped");
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        turned_away(&mut live, &mut a, PASSWORD, "KAT KAT KAT");
        let swapped = password::normalize(PASSWORD);
        assert_eq!(a.fields[GAMERTAG_FIELD], swapped);
        assert_eq!(a.form_error.as_deref(), Some("Wrong gamertag or password."));
        assert!(log_has(
            &said,
            "lobby: signing in to ws://127.0.0.1:9/live with a password"
        ));
        // With the PASSWORD box left empty, on a new PC: the server asks
        // for the password, and nothing said has what was typed.
        turned_away(&mut live, &mut a, PASSWORD, "");
        assert_eq!(a.fields[GAMERTAG_FIELD], swapped);
        assert!(log_has(&said, "lobby: signing in to ws://127.0.0.1:9/live"));
        // A setting changed now saves lobby.txt with the gamertag from
        // before (none, on a new PC), and the setting.
        a.set_control("vibration", "off").unwrap();
        let saved = Settings::load(&dir.join("lobby.txt"));
        assert_eq!(saved.gamertag, "");
        assert!(!saved.controls.vibration);
        // The right way round, the gamertag is saved once it's welcomed.
        fill_form(&mut a, "kat", PASSWORD);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live", "{:?}", a.form_error);
        assert_eq!(a.account(), Some(kat));
        assert_eq!(Settings::load(&dir.join("lobby.txt")).gamertag, "KAT");
        assert!(log_has(&said, "lobby: signed in to 127.0.0.1:9 as KAT"));
        drop(a);
        live.until(&mut [], |s, _| s.players_online() == 0);
        // On the PC KAT is signed in on now, the password typed in the
        // GAMERTAG box with the PASSWORD box empty: signed in as KAT, and
        // the notice doesn't say what was typed.
        let (mut a, said_again) = logged(&dir);
        a.input(Input::A);
        a.screen = Screen::SignIn;
        fill_form(&mut a, PASSWORD, "");
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live", "{:?}", a.form_error);
        assert_eq!(a.account(), Some(kat));
        assert!(!a.toasts.is_empty());
        for (toast, _) in &a.toasts {
            assert!(
                !toast.contains(&swapped) && !toast.contains(PASSWORD),
                "{toast}"
            );
        }
        // No log line and no file the lobby keeps has the password, in
        // either spelling.
        let log = said.lock().unwrap().join("\n") + &said_again.lock().unwrap().join("\n");
        for f in [PASSWORD, swapped.as_str()] {
            assert!(!log.contains(f), "{log}");
        }
        for file in std::fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            assert!(!holds(&bytes, &swapped), "{}", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rename_on_the_servers_command_line_follows_the_lobby() {
        let mut live = Live::new("renamed");
        let alpha = live.add("ALPHA", PASSWORD, false);
        let (mut a, _, dir) = new_pc(&mut live, "renamed", "alpha", PASSWORD);
        let rename = |gamertag: &str| Command::Rename {
            account: alpha,
            gamertag: gamertag.into(),
        };
        // Renamed while on the playlists: the lobby, and lobby.txt, follow.
        let done = live
            .server
            .command(Actor::Console, rename("ALPHA TWO"), live.now);
        assert_eq!(done.as_deref(), Ok("ALPHA IS ALPHA TWO NOW"));
        live.until(&mut [&mut a], |_, a| a[0].settings.gamertag == "ALPHA TWO");
        assert_eq!(a.saved[GAMERTAG_FIELD], "ALPHA TWO");
        assert_eq!(Settings::load(&dir.join("lobby.txt")).gamertag, "ALPHA TWO");
        drop(a);
        live.until(&mut [], |s, _| s.players_online() == 0);
        // Renamed while away: ONLINE (asking for the saved name) signs in
        // as the new one, with no word of typing the old one's password.
        let done = live
            .server
            .command(Actor::Console, rename("ALPHA 3"), live.now);
        assert!(done.is_ok());
        let (mut a, _) = logged(&dir);
        to_online(&mut a);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live");
        assert_eq!(a.account(), Some(alpha));
        assert_eq!(a.settings.gamertag, "ALPHA 3");
        assert_eq!(Settings::load(&dir.join("lobby.txt")).gamertag, "ALPHA 3");
        assert!(
            !a.toasts.iter().any(|t| t.0.contains("To use")),
            "{:?}",
            a.toasts
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_needed_opens_the_form_on_the_password() {
        let mut live = Live::new("password-needed");
        let alpha = live.add("ALPHA", PASSWORD, false);
        // A new folder whose lobby.txt names the gamertag.
        let dir = scratch("password-needed");
        let (mut a, said) = lobby(&dir, "ALPHA");
        to_online(&mut a);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.field, PASSWORD_FIELD);
        assert_eq!(a.fields[GAMERTAG_FIELD], "ALPHA");
        assert_eq!(
            a.form_error.as_deref(),
            Some("This PC isn't signed in to that account. Type your password.")
        );
        assert!(a.client.is_none() && a.signing_in.is_none());
        // The password typed there signs in.
        type_in(&mut a, PASSWORD);
        a.input(Input::A);
        live.sign_in(&mut a);
        assert_eq!(a.label(), "live");
        assert!(log_has(&said, "with a password"));
        assert_eq!(live.server.devices_of(alpha).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sign in with `typed` from the form, and be turned away.
    fn turned_away(live: &mut Live, a: &mut App, gamertag: &str, typed: &str) {
        fill_form(a, gamertag, typed);
        live.sign_in(a);
        assert_eq!(a.label(), "signin");
        assert!(a.password.is_empty());
    }

    #[test]
    fn a_wrong_password_clears_it_and_says_so() {
        let mut live = Live::new("wrong-password");
        let alpha = live.add("ALPHA", PASSWORD, false);
        let dir = scratch("wrong-password");
        let (mut a, _said) = lobby(&dir, "");
        to_online(&mut a);
        turned_away(&mut live, &mut a, "alpha", "NOT-THE-PASSWORD");
        assert_eq!(a.form_error.as_deref(), Some("Wrong gamertag or password."));
        assert_eq!(a.field, PASSWORD_FIELD);
        assert_eq!(a.fields[GAMERTAG_FIELD], "ALPHA");
        // A gamertag the server doesn't have says the same.
        turned_away(&mut live, &mut a, "nobody", PASSWORD);
        assert_eq!(a.form_error.as_deref(), Some("Wrong gamertag or password."));
        // Four more wrong for ALPHA, and even the right one waits.
        for _ in 0..4 {
            turned_away(&mut live, &mut a, "alpha", "STILL-NOT-IT");
        }
        turned_away(&mut live, &mut a, "alpha", PASSWORD);
        assert_eq!(
            a.form_error.as_deref(),
            Some("Too many wrong passwords. Try again in 10 minutes.")
        );
        assert!(live.server.devices_of(alpha).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_many_pcs_and_too_many_tries_show_on_the_form() {
        let dir = scratch("refusals");
        let (mut a, said) = lobby(&dir, "ALPHA");
        a.input(Input::A);
        let cases = [
            (
                TOO_MANY_PCS,
                None,
                "This account is on too many PCs. Ask the server's owner to sign it out of all PCs.",
            ),
            (
                TOO_MANY_TRIES,
                None,
                "Too many wrong passwords. Try again in 10 minutes.",
            ),
            (
                PASSWORD_NEEDED,
                Some(PASSWORD_FIELD),
                "This PC isn't signed in to that account. Type your password.",
            ),
            (
                WRONG_PASSWORD,
                Some(PASSWORD_FIELD),
                "Wrong gamertag or password.",
            ),
            (
                SIGNED_OUT,
                Some(PASSWORD_FIELD),
                "You were signed out of this PC. Type your password.",
            ),
        ];
        for (why, field, text) in cases {
            a.screen = Screen::Main;
            a.field = SERVER_FIELD;
            a.input(Input::A);
            assert_eq!(a.label(), "connecting", "{why}");
            // As if typed on the form; the dial is through.
            a.password = Secret::new(PASSWORD);
            a.dialing = None;
            a.on_event(LiveEvent::Refused(why.into()));
            assert_eq!(a.label(), "signin", "{why}");
            assert_eq!(a.form_error.as_deref(), Some(text));
            assert_eq!(a.field, field.unwrap_or(SERVER_FIELD), "{why}");
            assert!(a.password.is_empty());
            assert!(a.client.is_none() && a.signing_in.is_none() && a.dialing.is_none());
            assert!(log_has(&said, text));
        }
        // Signed out while on the playlists: the form, for the password.
        let (mut b, _server) = live_app(&dir);
        b.on_event(LiveEvent::Refused(SIGNED_OUT.into()));
        assert_eq!(b.label(), "signin");
        assert_eq!(b.field, PASSWORD_FIELD);
        // On the main menu, the player stays there and is told.
        let (mut b, _server) = live_app(&dir);
        b.input(Input::B);
        assert_eq!(b.label(), "main");
        b.on_event(LiveEvent::Refused(SIGNED_OUT.into()));
        assert_eq!(b.label(), "main");
        assert!(b.client.is_none());
        assert!(toasted(
            &b,
            "You were signed out of this PC. Type your password."
        ));
        // During a match, the form waits for the engine to close.
        #[cfg(unix)]
        {
            let (mut b, _server) = live_app(&dir);
            playing(&mut b, &dir);
            b.on_event(LiveEvent::Refused(SIGNED_OUT.into()));
            assert_eq!(b.label(), "ingame");
            assert!(b.form_after_match);
            b.tick(Instant::now());
            assert_eq!(b.label(), "ingame");
            let g = b.game.as_mut().unwrap();
            g.child.as_mut().unwrap().kill();
            let until = Instant::now() + Duration::from_secs(10);
            while b.label() == "ingame" && Instant::now() < until {
                b.tick(Instant::now());
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(b.label(), "signin");
            assert_eq!(b.field, PASSWORD_FIELD);
            assert_eq!(
                b.form_error.as_deref(),
                Some("You were signed out of this PC. Type your password.")
            );
            assert!(!b.form_after_match && b.game.is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_refusal_shows_whole_on_the_form() {
        let dir = scratch("long-refusal");
        let (mut a, _said) = lobby(&dir, "ALPHA");
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        a.dialing = None;
        a.on_event(LiveEvent::Refused(TOO_MANY_PCS.into()));
        assert_eq!(a.label(), "signin");
        if drew(&mut a) {
            // On two lines (wider than the form, it was cut), what to do
            // next too.
            let text = a.form_error.clone().unwrap();
            let d = a.drawn();
            let one = d.contains(&text);
            let two = d.windows(2).any(|w| format!("{} {}", w[0], w[1]) == text);
            assert!(one || two, "{d:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An engine playing a match for `a`: a script that sleeps until it's
    /// stopped, whatever it's asked.
    #[cfg(unix)]
    fn playing(a: &mut App, dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("engine.sh");
        std::fs::write(&exe, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let player = |relay_id: u64, account: u64, tag: &str| h2net::live::LauncherPlayer {
            relay_id,
            account,
            gamertag: tag.into(),
            team: 0,
            level: 4,
            party: account,
        };
        let m = LauncherMatch {
            id: 0x77,
            playlist: 11,
            ranked: true,
            teams: false,
            map: "lockout".into(),
            variant: "H2_FFA_HeadtoHead_b".into(),
            relay: h2net::live::RelaySeat {
                port: 47050,
                room: 0x77,
                id: 0x11,
                key: [0; 16],
            },
            host: 0x11,
            countdown: 20,
            players: vec![player(0x11, 1, "ALPHA"), player(0x22, 2, "BRAVO")],
        };
        let (session, me) = live::session_from(&m, "127.0.0.1".parse().unwrap()).unwrap();
        let engine = Match {
            kind: Kind::Fake,
            map: m.map.clone(),
            variant: m.variant.clone(),
            session: session.clone(),
            me,
            gamertag: "ALPHA".into(),
            instance: None,
            controls: Controls::default(),
            game_kind: child::game_kind(&m),
        };
        // A script just written can be busy for a moment, while another
        // test starts a process.
        let child = (0..50)
            .find_map(|_| match Running::start(&exe, dir, &engine, "") {
                Ok(c) => Some(c),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(20));
                    None
                }
            })
            .expect("the engine script starts");
        a.game = Some(Game {
            m,
            session,
            me,
            told: Told::default(),
            launch: false,
            start_at: a.now,
            child: Some(child),
            running: true,
            loaded: true,
            results: None,
            ended_at: None,
            over: None,
            left: None,
            menu: true,
            engine_controls: None,
        });
        a.screen = Screen::InGame;
    }

    #[test]
    fn a_changed_server_key_sends_nothing_and_asks() {
        let dir = scratch("key-changed");
        // The key pinned for the server, from last time.
        let was = [0x5A; 32];
        let mut pins = Pins::default();
        pins.set(SERVER, was);
        pins.save(&dir).unwrap();
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        // A server with another key: nothing but the LOGIN reaches it.
        let changed = |a: &mut App| {
            fill_form(a, "alpha", PASSWORD);
            let mut server = ByHand::answer(a, 3);
            let heard = server.heard();
            assert!(matches!(heard[..], [(ToServer::Login(_), _)]), "{heard:?}");
            server.challenge();
            a.tick(Instant::now());
            assert!(server.heard().is_empty());
            assert_eq!(a.label(), "keychanged");
            assert!(a.client.is_none() && a.dialing.is_none() && a.signing_in.is_none());
            server
        };
        let server = changed(&mut a);
        let now = server.key();
        assert_eq!(a.key_changed, Some((SERVER.to_string(), was, now)));
        assert!(log_has(&said, "instead of"));
        assert!(log_has(&said, "only the sign-in request was sent"));
        if drew(&mut a) {
            assert!(drawn(&a, "SERVER KEY CHANGED"));
            assert!(drawn(&a, &seal::short_code(&now)));
            assert!(drawn(&a, &seal::short_code(&was)));
            assert!(drawn(&a, "Trust the new key"));
        }
        // B: the main menu, the pin as it was, and the password forgotten.
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert!(a.key_changed.is_none() && a.password.is_empty());
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(was));
        // Again, and A this time: the new key is pinned, and the lobby
        // signs in again with the password it kept. (Not by a press that
        // comes as the popup opens: Enter held, or pressed twice.)
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        changed(&mut a);
        assert!(!a.password.is_empty());
        a.input(Input::A);
        a.key_repeated(Input::A);
        assert_eq!(a.label(), "keychanged");
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(was));
        a.tick(Instant::now() + TRUST_WAIT);
        a.key_repeated(Input::A);
        assert_eq!(a.label(), "keychanged");
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(now));
        assert_eq!(a.pins.get(SERVER), Some(now));
        assert!(log_has(&said, "lobby: trusting 127.0.0.1:9's new key"));
        let mut server = ByHand::answer(&mut a, 3);
        server.heard();
        server.challenge();
        a.tick(Instant::now());
        let heard = server.heard();
        assert!(
            matches!(heard[..], [(ToServer::ProvePassword { .. }, _)]),
            "{heard:?}"
        );
        server.welcome("ALPHA");
        a.tick(Instant::now());
        assert_eq!(a.label(), "live");
        assert!(a.password.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn server_keys_that_cannot_be_read_stop_the_sign_in_and_are_kept() {
        let dir = scratch("pins-unreadable");
        // Saved as UTF-16 (by an editor, say): it doesn't read.
        let file = crate::pins::path(&dir);
        let unreadable = [0xFF, 0xFE, b'h', 0, b'2', 0];
        std::fs::write(&file, unreadable).unwrap();
        let (mut a, said) = lobby(&dir, "");
        to_online(&mut a);
        fill_form(&mut a, "alpha", PASSWORD);
        // Nothing goes to the server (no key could be checked), it says
        // why, and the file is left as it was.
        let mut server = ByHand::answer(&mut a, 3);
        assert!(server.heard().is_empty());
        assert_eq!(a.label(), "failed");
        assert!(a.failed.starts_with("The server keys at "), "{}", a.failed);
        assert!(log_has(&said, "The server keys at "));
        assert!(a.client.is_none() && a.password.is_empty());
        assert_eq!(std::fs::read(&file).unwrap(), unreadable);
        // Once it reads, A signs in, and the key is pinned.
        std::fs::remove_file(&file).unwrap();
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        let mut server = ByHand::answer(&mut a, 3);
        let heard = server.heard();
        assert!(matches!(heard[..], [(ToServer::Login(_), _)]), "{heard:?}");
        server.challenge();
        a.tick(Instant::now());
        server.welcome("ALPHA");
        a.tick(Instant::now());
        assert_eq!(a.label(), "live");
        assert_eq!(Pins::load(&dir).unwrap().get(SERVER), Some(server.key()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_accounts_row_shows_once_accounts_arrive_and_says_my_account_for_players() {
        let mut live = Live::new("accounts-row");
        live.add("ALPHA", PASSWORD, false);
        live.add("ROOT", ROOT_PASSWORD, true);
        let (mut a, said, dir) = new_pc(&mut live, "accounts-row-player", "alpha", PASSWORD);
        let n = a.view().unwrap().playlists.len();
        // Before the accounts come, Settings is the last row.
        let kept = view(&mut a).accounts.take();
        assert_eq!(a.accounts_row(), None);
        for _ in 0..n + 5 {
            a.input(Input::Down);
        }
        assert_eq!(a.sel, n + 1);
        // Then My account is, for a player.
        view(&mut a).accounts = kept;
        assert_eq!(a.accounts_row(), Some(n + 2));
        a.input(Input::Down);
        assert_eq!(a.sel, n + 2);
        a.input(Input::Down);
        assert_eq!(a.sel, n + 2);
        if drew(&mut a) {
            assert!(a.drawn().iter().any(|d| d == "My account"));
            assert!(drawn(&a, "Change your password."));
        }
        a.input(Input::A);
        assert_eq!(a.label(), "accounts");
        // A player's screen: Change my password alone, and no list asked.
        assert_eq!(a.account_rows(), [Row::Change]);
        a.input(Input::Down);
        assert_eq!(a.acc.sel, 0);
        assert!(!log_has(&said, "lobby: asked for the accounts"));
        if drew(&mut a) {
            assert!(a.drawn().iter().any(|d| d == "MY ACCOUNT"));
            assert!(drawn(&a, "Change my password"));
            assert!(drawn(&a, "This is you."));
            assert!(!drawn(&a, "Create account"));
        }
        a.input(Input::B);
        assert_eq!(a.label(), "live");
        assert_eq!(a.sel, n + 2);
        // An admin's: Accounts, with every account, asked for again when
        // it opens.
        let (mut r, said, dir_r) = new_pc(&mut live, "accounts-row-admin", "root", ROOT_PASSWORD);
        assert_eq!(r.accounts_label(), "Accounts");
        open_accounts_row(&mut r);
        assert!(log_has(&said, "lobby: asked for the accounts"));
        live.until(&mut [&mut a, &mut r], |_, a| a[1].account_rows().len() == 4);
        let alpha = live.server.find("ALPHA").unwrap();
        assert_eq!(
            r.account_rows()[..3],
            [Row::Change, Row::Create, Row::Account(alpha)]
        );
        if drew(&mut r) {
            assert!(r.drawn().iter().any(|d| d == "ACCOUNTS"));
            for t in [
                "Create account",
                "ALPHA",
                "ROOT",
                "ADMIN",
                "ONLINE",
                "(YOU)",
                "1 PC",
            ] {
                assert!(drawn(&r, t), "{t}: {:?}", r.drawn());
            }
        }
        // No longer an admin: Change my password alone again.
        r.input(Input::Down);
        r.input(Input::Down);
        assert_eq!(r.acc.account, Some(alpha));
        view(&mut r).accounts.as_mut().unwrap().admin = false;
        r.tick(Instant::now());
        assert_eq!(r.account_rows(), [Row::Change]);
        assert_eq!((r.acc.sel, r.acc.account), (0, None));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir_r);
    }

    #[test]
    fn an_admin_creates_an_account_and_a_second_app_signs_in_with_the_password_shown() {
        let mut live = Live::new("create");
        live.add("ROOT", ROOT_PASSWORD, true);
        let (mut a, said, dir) = new_pc(&mut live, "create-admin", "root", ROOT_PASSWORD);
        open_accounts_row(&mut a);
        a.input(Input::Down);
        assert_eq!(a.acc.sel, 1);
        a.input(Input::A);
        assert_eq!(a.label(), "newaccount");
        // The gamertag first, typed (letters aren't buttons here).
        assert!(a.typing());
        let first = a.acc.password.as_str().to_string();
        assert!(password::check(&first).is_ok());
        type_in(&mut a, "root");
        assert_eq!(a.acc.gamertag, "ROOT");
        // The password row isn't typed in; its A makes another.
        a.input(Input::Down);
        assert!(!a.typing());
        a.input(Input::A);
        let made = a.acc.password.as_str().to_string();
        assert_ne!(made, first);
        // A taken gamertag: the server says so, in the popup.
        a.input(Input::Down);
        a.input(Input::A);
        assert!(a.acc.pending.is_some());
        if drew(&mut a) {
            assert!(drawn(&a, "WORKING..."));
        }
        // One request at a time: A waits.
        a.input(Input::A);
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        assert_eq!(a.label(), "newaccount");
        assert_eq!(a.acc.error.as_deref(), Some(GAMERTAG_TAKEN));
        // The cursor is back on the gamertag, to change it.
        assert_eq!(a.acc.row, NEW_GAMERTAG);
        assert!(a.typing());
        // A gamertag that breaks the rules isn't sent.
        for _ in 0..4 {
            a.input(Input::Backspace);
        }
        a.input(Input::A);
        assert_eq!(a.acc.row, 2);
        a.input(Input::A);
        assert!(a.acc.pending.is_none());
        assert_eq!(a.acc.row, 0);
        assert!(a.acc.error.is_some());
        // One starting with a number: the rule it breaks is shown in full.
        type_in(&mut a, "1kat");
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(a.acc.error.as_deref(), Some(password::GAMERTAG_RULES));
        if drew(&mut a) {
            assert!(drawn(&a, "STARTING WITH A LETTER"), "{:?}", a.drawn());
        }
        for _ in 0..4 {
            a.input(Input::Backspace);
        }
        // BRAVO, then.
        type_in(&mut a, "bravo");
        assert!(a.acc.error.is_none());
        a.input(Input::A);
        a.input(Input::A);
        assert!(log_has(&said, "lobby: asked to create account BRAVO"));
        live.until(&mut [&mut a], |_, a| a[0].popup == Some(Popup::NewPassword));
        let shown = a.acc.shown.as_ref().unwrap();
        assert_eq!(shown.gamertag, "BRAVO");
        assert!(!shown.reset);
        let given = shown.password.as_str().to_string();
        assert_eq!(given, made);
        let bravo = live.server.find("BRAVO").unwrap();
        if drew(&mut a) {
            assert!(a.drawn().iter().any(|d| d == "ACCOUNT CREATED"));
            assert!(drawn(&a, "BRAVO can sign in now with the password"));
            assert!(a.drawn().contains(&given));
            assert!(drawn(&a, "it won't be shown again"));
        }
        // A press as it opens (CREATE pressed once more, a B to leave)
        // leaves it up: it's the only time the password shows.
        a.input(Input::A);
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::NewPassword));
        assert!(a.acc.shown.is_some());
        // A once it has been up a moment: it's gone, and the new account
        // is selected.
        a.tick(Instant::now() + TRUST_WAIT);
        a.input(Input::A);
        assert!(a.popup.is_none() && a.acc.shown.is_none());
        live.until(&mut [&mut a], |_, a| a[0].account_rows().len() == 4);
        assert_eq!(a.acc.account, Some(bravo));
        assert_eq!(a.account_rows()[a.acc.sel], Row::Account(bravo));
        // Never in the log.
        assert!(!log_has(&said, &given) && !log_has(&said, &first));
        // A second PC signs in as BRAVO with the password shown.
        let (b, _, dir_b) = new_pc(&mut live, "create-bravo", "bravo", &given);
        assert_eq!(b.account(), Some(bravo));
        assert_eq!(live.server.devices_of(bravo).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    /// From the accounts screen, `who`'s option `k`, then A on the
    /// confirmation.
    fn confirm(a: &mut App, who: &str, k: usize) {
        assert_eq!(a.label(), "accounts");
        assert!(a.pick(who));
        a.input(Input::A);
        assert_eq!(a.label(), "accountoptions");
        for _ in 0..k {
            a.input(Input::Down);
        }
        a.input(Input::A);
        assert_eq!(a.label(), "confirmaccount");
        a.input(Input::A);
        assert!(a.acc.pending.is_some());
    }

    #[test]
    fn give_a_new_password_sign_out_and_remove_send_the_right_requests() {
        let mut live = Live::new("admin-actions");
        live.add("ROOT", ROOT_PASSWORD, true);
        let bravo = live.add("BRAVO", PASSWORD, false);
        let (mut b, _, dir_b) = new_pc(&mut live, "actions-bravo", "bravo", PASSWORD);
        let (mut a, said, dir) = new_pc(&mut live, "actions-root", "root", ROOT_PASSWORD);
        open_accounts_row(&mut a);
        live.until(&mut [&mut a, &mut b], |_, a| a[0].account_rows().len() == 4);
        // B on the confirmation goes back to the options.
        assert!(a.pick("bravo"));
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(
            a.popup,
            Some(Popup::ConfirmAccount(bravo, AccountAction::NewPassword))
        );
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::AccountOptions(bravo)));
        a.input(Input::B);
        // Give a new password: shown once, and BRAVO's PC stays signed in.
        confirm(&mut a, "bravo", 0);
        assert!(log_has(&said, "lobby: asked to give BRAVO a new password"));
        live.until(&mut [&mut a, &mut b], |_, a| {
            a[0].popup == Some(Popup::NewPassword)
        });
        let shown = a.acc.shown.as_ref().unwrap();
        assert!(shown.reset && shown.gamertag == "BRAVO");
        let given = shown.password.as_str().to_string();
        if drew(&mut a) {
            assert!(a.drawn().iter().any(|d| d == "NEW PASSWORD"));
            assert!(drawn(&a, "BRAVO's new password is"));
        }
        a.tick(Instant::now() + TRUST_WAIT);
        a.input(Input::A);
        assert!(a.popup.is_none());
        assert_eq!(b.label(), "live");
        assert_eq!(live.server.devices_of(bravo).len(), 1);
        // Sign out of all PCs: BRAVO's PC is back on the form.
        confirm(&mut a, "bravo", 1);
        assert!(log_has(&said, "lobby: asked to sign BRAVO out of all PCs"));
        live.until(&mut [&mut a, &mut b], |_, a| {
            a[0].acc.pending.is_none() && a[1].label() == "signin"
        });
        assert!(a.popup.is_none());
        assert!(live.server.devices_of(bravo).is_empty());
        assert_eq!(b.field, PASSWORD_FIELD);
        assert_eq!(
            b.form_error.as_deref(),
            Some("You were signed out of this PC. Type your password.")
        );
        // Where it signs in again, with the new password.
        type_in(&mut b, &given);
        b.input(Input::A);
        live.until(&mut [&mut a, &mut b], |_, a| a[1].label() == "live");
        assert_eq!(live.server.devices_of(bravo).len(), 1);
        // Remove: the account and its PCs are gone, and so is BRAVO.
        confirm(&mut a, "bravo", 2);
        assert!(log_has(&said, "lobby: asked to remove account BRAVO"));
        live.until(&mut [&mut a, &mut b], |_, a| {
            a[0].acc.pending.is_none() && a[1].client.is_none()
        });
        assert!(live.server.find("BRAVO").is_none());
        assert!(live.server.devices_of(bravo).is_empty());
        assert_eq!(b.label(), "failed");
        assert!(b.failed.contains(ACCOUNT_REMOVED), "{}", b.failed);
        live.until(&mut [&mut a], |_, a| a[0].account_rows().len() == 3);
        assert!(a.popup.is_none());
        assert!(!a.account_rows().contains(&Row::Account(bravo)));
        assert!(a.acc.sel < 3);
        assert!(!log_has(&said, &given));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn a_new_password_the_server_cannot_save_says_so_in_full() {
        let mut live = Live::new("unsaved-password");
        live.add("ROOT", ROOT_PASSWORD, true);
        live.add("BRAVO", PASSWORD, false);
        let (mut a, said, dir) = new_pc(&mut live, "unsaved-root", "root", ROOT_PASSWORD);
        open_accounts_row(&mut a);
        live.until(&mut [&mut a], |_, a| a[0].account_rows().len() == 4);
        // The server can't replace logins.txt (a folder is where its
        // temporary file would go).
        let blocked = live.dir.join("logins.txt.tmp");
        std::fs::create_dir(&blocked).unwrap();
        confirm(&mut a, "bravo", 0);
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        // Refused: no password is shown, and all the server said is, its
        // last words too.
        assert_eq!(a.label(), "confirmaccount");
        assert!(a.acc.shown.is_none());
        let error = a.acc.error.clone().unwrap();
        assert!(error.starts_with("BRAVO HAS A NEW PASSWORD."), "{error}");
        assert!(error.ends_with("TRY AGAIN."), "{error}");
        if drew(&mut a) {
            assert!(drawn(&a, "BRAVO HAS A NEW PASSWORD."), "{:?}", a.drawn());
            assert!(drawn(&a, "TRY AGAIN."), "{:?}", a.drawn());
        }
        // A again once it can be saved: a new password, shown.
        std::fs::remove_dir(&blocked).unwrap();
        a.input(Input::A);
        live.until(&mut [&mut a], |_, a| a[0].popup == Some(Popup::NewPassword));
        let given = a.acc.shown.as_ref().unwrap().password.as_str().to_string();
        assert!(!log_has(&said, &given));
        let (b, _, dir_b) = new_pc(&mut live, "unsaved-bravo", "bravo", &given);
        assert_eq!(b.label(), "live");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    fn row(account: u64, gamertag: &str, flags: u8) -> AccountRow {
        AccountRow {
            account,
            gamertag: gamertag.into(),
            flags,
            pcs: 2,
            best: 7,
        }
    }

    /// What an admin (ALPHA, account 1) is told of the accounts.
    fn admins_accounts() -> Accounts {
        Accounts {
            admin: true,
            password: true,
            answers: 0,
            ok: true,
            text: String::new(),
            total: 3,
            rows: vec![
                row(1, "ALPHA", ROW_ADMIN | ROW_PASSWORD | ROW_ONLINE),
                row(2, "BRAVO", ROW_PASSWORD),
                row(3, "ZETA", ROW_ADMIN | ROW_PASSWORD),
            ],
        }
    }

    /// A match for ALPHA (account 1, this PC) and BRAVO.
    fn a_match() -> LauncherMatch {
        let player = |relay_id: u64, account: u64, tag: &str| h2net::live::LauncherPlayer {
            relay_id,
            account,
            gamertag: tag.into(),
            team: 0,
            level: 4,
            party: account,
        };
        LauncherMatch {
            id: 0x78,
            playlist: 11,
            ranked: true,
            teams: false,
            map: "lockout".into(),
            variant: "H2_FFA_HeadtoHead_b".into(),
            relay: h2net::live::RelaySeat {
                port: 47050,
                room: 0x78,
                id: 0x11,
                key: [0; 16],
            },
            host: 0x11,
            countdown: 20,
            players: vec![player(0x11, 1, "ALPHA"), player(0x22, 2, "BRAVO")],
        }
    }

    /// NEW PASSWORD up on `a`, showing BRAVO's (long enough for A and B
    /// to close it).
    fn showing_a_password(a: &mut App) {
        a.screen = Screen::Accounts;
        a.acc.shown = Some(Shown {
            gamertag: "BRAVO".into(),
            password: Secret::new(PASSWORD),
            reset: true,
            at: a.now.checked_sub(TRUST_WAIT).unwrap(),
        });
        a.popup = Some(Popup::NewPassword);
    }

    #[test]
    fn a_new_password_stays_up_until_the_player_closes_it() {
        let dir = scratch("password-stays");
        // Over a real connection on this PC: a match needs the server's
        // address.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let pc = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let ours = Connection::tcp(pc).unwrap();
        let theirs = Connection::tcp(server).unwrap();
        let (mut a, _theirs) = live_app_on(&dir, ours, theirs);
        // A match starts: the pregame, under the password.
        showing_a_password(&mut a);
        a.on_event(LiveEvent::LauncherMatch(a_match()));
        assert_eq!(a.screen, Screen::Pregame);
        assert_eq!(a.popup, Some(Popup::NewPassword));
        assert!(a.acc.shown.is_some());
        a.input(Input::A);
        assert!(a.popup.is_none() && a.acc.shown.is_none());
        assert_eq!(a.screen, Screen::Pregame);
        // The server is lost: the failure screen, under the password.
        let (mut a, _server) = live_app(&dir);
        showing_a_password(&mut a);
        a.on_event(LiveEvent::Lost("gone".into()));
        a.tick(Instant::now());
        assert_eq!(a.screen, Screen::Failed);
        assert_eq!(a.popup, Some(Popup::NewPassword));
        // Signed out: the form, under the password.
        let (mut a, _server) = live_app(&dir);
        showing_a_password(&mut a);
        a.on_event(LiveEvent::Refused(SIGNED_OUT.into()));
        a.tick(Instant::now());
        assert_eq!(a.screen, Screen::SignIn);
        assert_eq!(a.popup, Some(Popup::NewPassword));
        a.input(Input::B);
        assert!(a.popup.is_none() && a.acc.shown.is_none());
        assert_eq!(a.label(), "signin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_password_is_not_closed_by_a_press_meant_for_the_screen_before_it() {
        let dir = scratch("password-early");
        let (mut a, _server) = live_app(&dir);
        a.screen = Screen::Accounts;
        let start = a.now;
        a.show_password("BRAVO".into(), Secret::new(PASSWORD), true);
        // CREATE or Give new password pressed again, or B pressed to leave,
        // just as the answer comes: the password stays up.
        for i in [Input::A, Input::B, Input::A] {
            a.input(i);
            assert_eq!(a.popup, Some(Popup::NewPassword));
            let shown = a.acc.shown.as_ref().map(|s| s.password.as_str());
            assert_eq!(shown, Some(PASSWORD));
        }
        a.now = start + TRUST_WAIT / 2;
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::NewPassword));
        // Once it has been up a moment, either closes it (and forgets it).
        a.now = start + TRUST_WAIT;
        a.input(Input::B);
        assert!(a.popup.is_none() && a.acc.shown.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_answer_with_a_list_after_it_in_the_same_poll_is_not_lost() {
        let mut live = Live::new("answer-then-list");
        live.add("ROOT", ROOT_PASSWORD, true);
        live.add("BRAVO", PASSWORD, false);
        let (mut a, _, dir) = new_pc(&mut live, "answer-root", "root", ROOT_PASSWORD);
        open_accounts_row(&mut a);
        live.until(&mut [&mut a], |_, a| a[0].account_rows().len() == 4);
        confirm(&mut a, "bravo", 0);
        // The request goes; the server answers it and then, before the
        // lobby looks, the command line makes an account: that list
        // (answering nothing) comes after the answer, in the same poll.
        a.tick(Instant::now());
        live.now += 0.05;
        live.server.poll(live.now);
        live.add("CHARLIE", PASSWORD, false);
        live.now += 0.05;
        live.server.poll(live.now);
        a.tick(Instant::now());
        assert_eq!(a.accounts().map(|v| v.answers), Some(0));
        assert!(a.acc.pending.is_none());
        assert_eq!(a.popup, Some(Popup::NewPassword));
        assert_eq!(a.acc.shown.as_ref().unwrap().gamertag, "BRAVO");
        assert_eq!(a.account_rows().len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_request_unanswered_for_ten_seconds_is_given_up_and_the_list_asked_for() {
        let mut live = Live::new("unanswered");
        live.add("ROOT", ROOT_PASSWORD, true);
        let bravo = live.add("BRAVO", PASSWORD, false);
        let (mut a, said, dir) = new_pc(&mut live, "unanswered-root", "root", ROOT_PASSWORD);
        open_accounts_row(&mut a);
        live.until(&mut [&mut a], |_, a| a[0].account_rows().len() == 4);
        let lines = |said: &Said, start: &str| {
            let said = said.lock().unwrap();
            said.iter().filter(|l| l.starts_with(start)).count()
        };
        // (The list the screen asked for as it opened is in.)
        for _ in 0..5 {
            live.step(&mut [&mut a]);
        }
        let lists = lines(&said, "lobby: asked for the accounts");
        let answers = lines(&said, "live: accounts (answering ")
            - lines(&said, "live: accounts (answering 0");
        // Sign BRAVO out, and the server doesn't answer for ten seconds.
        confirm(&mut a, "bravo", 1);
        a.tick(Instant::now() + ASK_WAIT + Duration::from_secs(1));
        assert!(a.acc.pending.is_none());
        assert!(toasted(&a, "No answer from the server. Check the list."));
        assert_eq!(lines(&said, "lobby: asked for the accounts"), lists + 1);
        // The late answers (to it, and to the list) change nothing.
        for _ in 0..5 {
            live.step(&mut [&mut a]);
        }
        let now = lines(&said, "live: accounts (answering ")
            - lines(&said, "live: accounts (answering 0");
        assert_eq!(now, answers + 2);
        assert!(!a.toasts.iter().any(|t| t.0.contains("SIGNED OUT")));
        // The confirmation is still up, and may be sent again.
        assert_eq!(a.label(), "confirmaccount");
        a.input(Input::A);
        assert!(a.acc.pending.is_some());
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        assert!(a.popup.is_none());
        assert!(toasted(&a, "BRAVO IS SIGNED OUT OF ALL PCS."));
        assert!(live.server.devices_of(bravo).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_on_your_own_or_an_admins_row_are_dim() {
        let dir = scratch("dim-options");
        let (mut a, _server) = live_app(&dir);
        view(&mut a).accounts = Some(admins_accounts());
        a.open_accounts();
        assert_eq!(a.label(), "accounts");
        let mine = "Your own account: use the server's command line.";
        let admins = "An admin's account: use the server's command line.";
        for (who, why) in [("alpha", mine), ("zeta", admins)] {
            assert!(a.pick(who));
            a.input(Input::A);
            assert_eq!(a.label(), "accountoptions");
            if drew(&mut a) {
                assert!(drawn(&a, why), "{who}");
                assert!(drawn(&a, "Give a new password"));
            }
            // Every option says to use the command line, and opens nothing.
            for _ in AccountAction::ALL {
                a.toasts.clear();
                a.input(Input::A);
                assert_eq!(a.label(), "accountoptions");
                assert!(toasted(&a, COMMAND_LINE), "{who}");
                a.input(Input::Down);
            }
            a.input(Input::B);
            assert_eq!(a.label(), "accounts");
        }
        // Another player's account isn't: each option asks first.
        assert!(a.pick("bravo"));
        a.input(Input::A);
        a.toasts.clear();
        if drew(&mut a) {
            assert!(!drawn(&a, "command line"));
        }
        let asks = [
            "Give BRAVO a new password? Their PCs stay signed in.",
            "Sign BRAVO out of all PCs?",
            "Remove BRAVO for good?",
        ];
        for (k, (action, ask)) in AccountAction::ALL.into_iter().zip(asks).enumerate() {
            assert_eq!(a.osel, k);
            a.input(Input::A);
            assert_eq!(a.popup, Some(Popup::ConfirmAccount(2, action)));
            let question = action.question("BRAVO");
            assert!(question.starts_with(ask));
            // Signed out or removed, a host's game ends for everyone in it,
            // and that's said.
            let everyone = "ends for them, or for everyone if they host it.";
            assert_eq!(
                question.ends_with(everyone),
                action != AccountAction::NewPassword
            );
            if drew(&mut a) {
                assert!(a.drawn().iter().any(|d| d == action.title()));
                // (Its last line, too.)
                let end = question.rsplit(' ').next().unwrap();
                assert!(a.drawn().iter().any(|d| d.ends_with(end)), "{end}");
            }
            a.input(Input::B);
            a.input(Input::Down);
        }
        // Up from the first goes round to the last.
        a.input(Input::Up);
        assert_eq!(a.osel, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn change_password_checks_locally_then_changes_it() {
        let mut live = Live::new("change-password");
        let alpha = live.add("ALPHA", PASSWORD, false);
        let (mut a, said, dir) = new_pc(&mut live, "change-alpha", "alpha", PASSWORD);
        open_accounts_row(&mut a);
        a.input(Input::A);
        assert_eq!(a.label(), "changepassword");
        assert!(a.typing());
        // A on empty fields moves on, then says what's missing.
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(a.acc.row, 2);
        a.input(Input::A);
        assert_eq!(a.acc.row, 0);
        assert_eq!(a.acc.error.as_deref(), Some("Type your current password."));
        assert!(a.acc.pending.is_none());
        // Typed (letters aren't buttons here); the new ones differ.
        let new = "new pass 4471";
        type_in(&mut a, "not the password");
        a.input(Input::Tab);
        type_in(&mut a, new);
        a.input(Input::Tab);
        type_in(&mut a, "new pass 4472");
        a.input(Input::A);
        assert_eq!(
            a.acc.error.as_deref(),
            Some("The new passwords don't match.")
        );
        assert_eq!(a.acc.row, 2);
        // Too short.
        let rub = |a: &mut App, row: usize| {
            a.acc.row = row;
            for _ in 0..2 * MAX_PASSWORD {
                a.input(Input::Backspace);
            }
        };
        rub(&mut a, 1);
        type_in(&mut a, "short");
        rub(&mut a, 2);
        type_in(&mut a, "SHORT");
        a.input(Input::A);
        assert_eq!(a.acc.row, 1);
        assert_eq!(
            a.acc.error.as_deref(),
            Some(password::check("short").unwrap_err())
        );
        assert!(a.acc.pending.is_none());
        // Drawn masked.
        if drew(&mut a) {
            assert!(a.drawn().iter().any(|d| d == "CHANGE PASSWORD"));
            assert!(a.drawn().iter().any(|d| d == "*****"));
            assert!(!drawn(&a, "short") && !drawn(&a, "SHORT") && !drawn(&a, "not the"));
        }
        // The rules kept, but the current password is wrong: the server
        // says so, in the popup.
        rub(&mut a, 1);
        type_in(&mut a, new);
        rub(&mut a, 2);
        type_in(&mut a, &new.to_uppercase());
        a.input(Input::A);
        assert!(a.acc.pending.is_some());
        assert!(log_has(&said, "lobby: asked to change our password"));
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        assert_eq!(a.label(), "changepassword");
        assert_eq!(a.acc.error.as_deref(), Some(WRONG_OLD_PASSWORD));
        // The wrong one is rubbed out, the cursor on it; the new ones stay.
        assert_eq!(a.acc.row, 0);
        assert!(a.acc.change[0].is_empty());
        assert!(!a.acc.change[1].is_empty() && !a.acc.change[2].is_empty());
        // The right one: changed, the popup closed.
        type_in(&mut a, &PASSWORD.to_lowercase());
        a.input(Input::A);
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        assert!(a.popup.is_none(), "{:?}", a.acc.error);
        assert!(toasted(&a, "Password changed."));
        assert!(a.acc.change.iter().all(Secret::is_empty));
        // This PC stays signed in, and nothing typed was logged.
        assert_eq!(a.label(), "accounts");
        assert_eq!(live.server.devices_of(alpha).len(), 1);
        for typed in [new, "NEWPASS4471", "not the password", "short", PASSWORD] {
            assert!(!log_has(&said, typed), "{typed}");
        }
        // A new PC signs in with the new password, and not the old.
        let (b, _, dir_b) = new_pc(&mut live, "change-new-pc", "alpha", "NEW-PASS-4471");
        assert_eq!(b.account(), Some(alpha));
        let dir_c = scratch("change-old-pc");
        let (mut c, _) = lobby(&dir_c, "");
        to_online(&mut c);
        turned_away(&mut live, &mut c, "alpha", PASSWORD);
        assert_eq!(c.form_error.as_deref(), Some("Wrong gamertag or password."));
        for d in [dir, dir_b, dir_c] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn a_password_change_the_server_cannot_save_is_tried_again_from_the_new_one() {
        let mut live = Live::new("change-unsaved");
        let alpha = live.add("ALPHA", PASSWORD, false);
        let (mut a, said, dir) = new_pc(&mut live, "change-unsaved-alpha", "alpha", PASSWORD);
        open_accounts_row(&mut a);
        a.input(Input::A);
        assert_eq!(a.label(), "changepassword");
        // The server can't replace logins.txt (a folder is where its
        // temporary file would go).
        let blocked = live.dir.join("logins.txt.tmp");
        std::fs::create_dir(&blocked).unwrap();
        let new = "NEW-PASS-4471";
        type_in(&mut a, PASSWORD);
        a.input(Input::Tab);
        type_in(&mut a, new);
        a.input(Input::Tab);
        type_in(&mut a, new);
        a.input(Input::A);
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        // Said in full, in the popup; the new password is the current one
        // now, as it is on the server.
        assert_eq!(a.label(), "changepassword");
        let error = a.acc.error.clone().unwrap();
        assert!(error.starts_with(PASSWORD_CHANGED), "{error}");
        assert!(error.ends_with("TRY AGAIN."), "{error}");
        assert_eq!(a.acc.change[0].as_str(), new);
        assert!(a.acc.change[1..].iter().all(|s| s.as_str() == new));
        // A again once it can be saved: changed (not WRONG PASSWORD), and
        // the popup closes.
        std::fs::remove_dir(&blocked).unwrap();
        a.input(Input::A);
        live.until(&mut [&mut a], |_, a| a[0].acc.pending.is_none());
        assert!(a.popup.is_none(), "{:?}", a.acc.error);
        assert!(toasted(&a, "Password changed."));
        assert!(a.acc.change.iter().all(Secret::is_empty));
        for typed in [new, "NEWPASS4471", PASSWORD] {
            assert!(!log_has(&said, typed), "{typed}");
        }
        // Saved: after a restart, a new PC signs in with the new password.
        live.server = Server::open(&live.dir, Some("test")).unwrap();
        live.server.set_sign_up(SignUp::Closed);
        live.server.set_password_cost(Cost::Cheap);
        let (b, _, dir_b) = new_pc(&mut live, "change-unsaved-b", "alpha", new);
        assert_eq!(b.account(), Some(alpha));
        for d in [dir, dir_b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn update_your_launcher_is_reworded() {
        let dir = scratch("update-launcher");
        let (mut a, _said) = lobby(&dir, "ALPHA");
        a.input(Input::A);
        let refused = |a: &mut App, why: &str| {
            a.screen = Screen::Main;
            a.input(Input::A);
            assert_eq!(a.label(), "connecting");
            a.dialing = None;
            a.on_event(LiveEvent::Refused(why.into()));
            assert_eq!(a.label(), "failed", "{why}");
        };
        refused(&mut a, UPDATE_YOUR_LAUNCHER);
        assert_eq!(
            a.failed,
            "The server runs a different version from this launcher. Ask its owner for the right one."
        );
        // The rest are as they were.
        for why in [SERVER_BUSY, ACCOUNT_REMOVED, NO_ACCOUNT] {
            refused(&mut a, why);
            assert_eq!(a.failed, format!("The server turned us away: {why}"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
