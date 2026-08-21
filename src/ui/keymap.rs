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
    Commit,
    FocusCommit,
    Discard,
    Unfocus,
    ClosePeek,
    PeekBack,
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

pub const HINTS: &str =
    "j/k files · s stage · n/p hunks · e collapse · t tests · d discard · c commit message · ⌘↩ commit · ⌘O open · ⌘click definition";

const GLOBAL_BINDINGS: &[(Modifiers, Key, Command)] = &[
    (Modifiers::COMMAND, Key::Enter, Command::Commit),
    (Modifiers::COMMAND, Key::O, Command::OpenRepo),
    (Modifiers::COMMAND, Key::R, Command::Refresh),
    (Modifiers::COMMAND, Key::OpenBracket, Command::PeekBack),
    (Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::A, Command::StageAll),
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
