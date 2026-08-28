use egui::{Context, Key, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NextFile,
    PrevFile,
    ToggleStage,
    NextHunk,
    PrevHunk,
    ToggleTests,
    ToggleCollapse,
    Refresh,
    StageAll,
    OpenRepo,
    OpenRange,
    Commit,
    FocusCommit,
    Discard,
    Unfocus,
    ClosePeek,
    PeekBack,
    SwitchTab(usize),
    NextTab,
    PrevTab,
    CloseTab,
}

pub fn poll(ctx: &Context) -> Vec<Command> {
    let mut commands = Vec::new();
    ctx.input_mut(|input| {
        for (modifiers, key, command) in GLOBAL_BINDINGS {
            if input.consume_key(*modifiers, *key) {
                commands.push(*command);
            }
        }
    });
    if ctx.egui_wants_keyboard_input() {
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            commands.push(Command::Unfocus);
        }
        return commands;
    }
    ctx.input(|input| {
        for (key, command) in PLAIN_BINDINGS {
            if input.key_pressed(*key) {
                commands.push(*command);
            }
        }
    });
    commands
}

pub fn hints(range: bool) -> String {
    let modifier = modifier_symbol();
    if range {
        return format!(
            "j/k files · s mark read · n/p hunks · e collapse · t tests · {modifier}⇧T range · {modifier}T open · {modifier}1-9 tabs · {modifier}W close tab · {modifier}click definition"
        );
    }
    format!(
        "j/k files · s stage · n/p hunks · e collapse · t tests · d discard · c commit message · {modifier}↩ commit · {modifier}⇧T range · {modifier}T open · {modifier}1-9 tabs · {modifier}W close tab · {modifier}click definition"
    )
}

pub fn key_label(key: &str) -> String {
    format!("{}{key}", modifier_symbol())
}

fn modifier_symbol() -> &'static str {
    if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" }
}

// Order matters: `consume_key` matches modifiers *logically*, ignoring extra
// Shift and Alt, so a shifted binding must be listed before its unshifted
// sibling or the plain one swallows it first and the shifted one never fires.
const GLOBAL_BINDINGS: &[(Modifiers, Key, Command)] = &[
    (Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::T, Command::OpenRange),
    (Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::A, Command::StageAll),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab, Command::PrevTab),
    (Modifiers::CTRL, Key::Tab, Command::NextTab),
    (Modifiers::COMMAND, Key::Enter, Command::Commit),
    (Modifiers::COMMAND, Key::O, Command::OpenRepo),
    (Modifiers::COMMAND, Key::T, Command::OpenRepo),
    (Modifiers::COMMAND, Key::W, Command::CloseTab),
    (Modifiers::COMMAND, Key::R, Command::Refresh),
    (Modifiers::COMMAND, Key::OpenBracket, Command::PeekBack),
    (Modifiers::COMMAND, Key::Num1, Command::SwitchTab(0)),
    (Modifiers::COMMAND, Key::Num2, Command::SwitchTab(1)),
    (Modifiers::COMMAND, Key::Num3, Command::SwitchTab(2)),
    (Modifiers::COMMAND, Key::Num4, Command::SwitchTab(3)),
    (Modifiers::COMMAND, Key::Num5, Command::SwitchTab(4)),
    (Modifiers::COMMAND, Key::Num6, Command::SwitchTab(5)),
    (Modifiers::COMMAND, Key::Num7, Command::SwitchTab(6)),
    (Modifiers::COMMAND, Key::Num8, Command::SwitchTab(7)),
    (Modifiers::COMMAND, Key::Num9, Command::SwitchTab(8)),
];

const PLAIN_BINDINGS: &[(Key, Command)] = &[
    (Key::J, Command::NextFile),
    (Key::ArrowDown, Command::NextFile),
    (Key::K, Command::PrevFile),
    (Key::ArrowUp, Command::PrevFile),
    (Key::S, Command::ToggleStage),
    (Key::Space, Command::ToggleStage),
    (Key::N, Command::NextHunk),
    (Key::P, Command::PrevHunk),
    (Key::T, Command::ToggleTests),
    (Key::E, Command::ToggleCollapse),
    (Key::R, Command::Refresh),
    (Key::C, Command::FocusCommit),
    (Key::D, Command::Discard),
    (Key::Escape, Command::ClosePeek),
];
