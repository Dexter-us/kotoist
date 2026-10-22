use std::sync::{mpsc, Arc, Mutex};

use egui;
use egui::containers::ScrollArea;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
use nih_plug::editor::Editor;

use self::piano_roll::PianoRoll;
use crate::parameters::{InterpreterMessage, Parameters, Snippet};
use crate::pipe::{Message as PipeMessage, PipeOut};

mod adapter;
mod geometry;
mod piano_roll;
mod resize;
mod script_files;
#[cfg(windows)]
mod windows_focus;
#[cfg(windows)]
mod windows_geometry;
#[cfg(windows)]
mod windows_native_resize;

pub(crate) use geometry::EditorGeometry;
pub(crate) use piano_roll::Event as PianoRollEvent;

// A compact first-open size keeps the code area and Run controls on screen in
// common Windows plugin hosts. Longer code and console output scroll instead
// of forcing the editor to start taller than the host window.
pub(crate) const WINDOW_SIZE: (u32, u32) = (560, 420);

pub(crate) fn create_editor(
    params: Arc<Parameters>,
    pipe_out: Arc<Mutex<PipeOut>>,
    piano_roll_receiver: mpsc::Receiver<Vec<PianoRollEvent>>,
) -> Option<Box<dyn Editor>> {
    let piano_roll = PianoRoll::new(
        piano_roll_receiver,
        params.clone_cursor(),
        params.clone_gl_context_validity(),
    );
    let geometry = Arc::new(Mutex::new(EditorGeometry::new(params.editor_state.size())));
    Some(Box::new(adapter::KotoistEditor::new(
        params,
        pipe_out,
        Arc::new(Mutex::new(piano_roll)),
        geometry,
    )))
}

fn draw_editor_ui(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    params: &Arc<Parameters>,
    pipe_out: &Arc<Mutex<PipeOut>>,
    piano_roll: &Arc<Mutex<PianoRoll>>,
) {
    let bg = ui.visuals().faint_bg_color;
    egui::Frame::default()
        .inner_margin(7.0)
        .fill(bg)
        .show(ui, |ui| {
            let width = ui.available_width();
            let height = ui.available_height();
            let tab_height = 42.0_f32.min(height);
            let console_height = 110.0_f32.min((height - tab_height).max(0.0));
            let editor_height = (height - tab_height - console_height).max(0.0);

            ui.allocate_ui_with_layout(
                egui::vec2(width, tab_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| tab_bar(ui, params.clone()),
            );

            ui.allocate_ui_with_layout(
                egui::vec2(width, editor_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        piano_roll.lock().unwrap().draw(ui);
                    });
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        let context = ui.ctx().clone();
                        text_editor(&context, ui, state, params);
                    });
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(width, console_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| bottom_panel(ui, state, params, pipe_out),
            );
        });

    script_files_dialog(ui.ctx(), state, params);
}

fn tab_bar(ui: &mut egui::Ui, params: Arc<Parameters>) {
    ScrollArea::horizontal().show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let selected_snippet = params.selected_snippet_index();
            if let Ok(snippets) = params.snippets.try_read() {
                for (index, snippet) in snippets.iter().enumerate() {
                    let fallback_index = index
                        .min(snippets.len().saturating_sub(2))
                        .min(selected_snippet);
                    show_tab(
                        ui,
                        snippet,
                        index,
                        fallback_index,
                        index == selected_snippet,
                        params.clone(),
                    );
                }
            }

            if ui.button("+").clicked() {
                params.send_interpreter_msg(InterpreterMessage::AddSnippet);
            }
        });
    });
}

fn show_tab(
    ui: &mut egui::Ui,
    snippet: &Snippet,
    index: usize,
    fallback_index: usize,
    selected: bool,
    params: Arc<Parameters>,
) {
    if ui
        .selectable_label(selected, egui::RichText::new(&snippet.name).size(22.0))
        .clicked()
    {
        params.set_selected_snippet_index(index);
    }

    if ui.selectable_label(false, "x").clicked() {
        params.send_interpreter_msg(InterpreterMessage::RemoveSnippet(index));
        params.set_selected_snippet_index(fallback_index);
    }
}

fn text_editor(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    params: &Arc<Parameters>,
) {
    ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(false)
        .show(ui, |ui| {
            // we need to sync the state
            {
                let index = params.selected_snippet_index();
                if let Some(snippet) = params
                    .snippets
                    .try_read()
                    .ok()
                    .as_ref()
                    .and_then(|s| s.get(index))
                {
                    if state.text_buffer != snippet.code {
                        state.text_buffer = snippet.code.clone();
                    }
                }
            }

            let syntax = koto_syntax();
            let (output, _) = CodeEditor::default()
                .id_source("code editor")
                .with_rows(10)
                .with_fontsize(13.0)
                .vscroll(true)
                .with_theme(ColorTheme::AYU_MIRAGE)
                .with_numlines(true)
                .show(ui, &mut state.text_buffer, &syntax);

            if output.response.changed() {
                let index = params.selected_snippet_index();
                params.send_interpreter_msg(InterpreterMessage::SetSnippetCode(
                    index,
                    state.text_buffer.clone(),
                ));
            }

            let selected_text = output
                .cursor_range
                .map(|text_cursor_range| {
                    use egui::TextBuffer as _;
                    let selected_chars = text_cursor_range.as_sorted_char_range();
                    state.text_buffer.char_range(selected_chars)
                })
                .unwrap_or_default();

            let clicked_inside_editor = ctx.input(|input| {
                input.pointer.button_pressed(egui::PointerButton::Primary)
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|position| output.response.rect.contains(position))
            });
            if clicked_inside_editor || output.response.clicked() {
                output.response.request_focus();
            }

            ctx.input_mut(|i| {
                // Cmd-Enter - eval all/selection
                if i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Enter,
                )) {
                    let code = if selected_text.is_empty() {
                        &state.text_buffer
                    } else {
                        selected_text
                    };

                    params.send_interpreter_msg(InterpreterMessage::EvalCode(code.to_owned()));
                }
            });
        });
}

fn bottom_panel(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    params: &Arc<Parameters>,
    pipe_out: &Arc<Mutex<PipeOut>>,
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label("Console");
            ui.small(format!("v{}", env!("CARGO_PKG_VERSION")));
            ui.add_space(6.0);
        });
        if ui.button("Load").clicked() {
            state.file_dialog = Some(ScriptDialog::Load);
            state.file_notice.clear();
        }
        if ui.button("Save").clicked() {
            state.file_dialog = Some(ScriptDialog::Save);
            state.file_name = "My Script".to_owned();
            state.file_notice.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("🚀 Run").clicked() {
                params
                    .send_interpreter_msg(InterpreterMessage::EvalCode(state.text_buffer.clone()));
            }
        });
    });
    ScrollArea::both().stick_to_bottom(true).show(ui, |ui| {
        let pipe = pipe_out.lock().unwrap();
        if let Ok(out) = pipe.receiver.try_recv() {
            state.console.push(out);
        }

        for out in &state.console {
            match out {
                PipeMessage::Normal(out) => {
                    let out = egui::RichText::new(out).monospace();
                    let _ = ui.label(out);
                }
                PipeMessage::Error(out) => {
                    let color = ui.visuals().error_fg_color;
                    let out = egui::RichText::new(out).monospace().color(color);
                    let _ = ui.label(out);
                }
            }
        }

        ui.allocate_space(ui.available_size());
    });
}

#[derive(Clone, Copy)]
enum ScriptDialog {
    Load,
    Save,
}

fn script_files_dialog(ctx: &egui::Context, state: &mut GuiState, params: &Arc<Parameters>) {
    let Some(mode) = state.file_dialog else {
        return;
    };

    let title = match mode {
        ScriptDialog::Load => "Load Koto script",
        ScriptDialog::Save => "Save Koto script",
    };
    let mut open = true;
    let mut close_after_action = false;

    egui::Window::new(title)
        .id(egui::Id::new("kotoist-script-files"))
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .open(&mut open)
        .show(ctx, |ui| {
            match script_files::directory() {
                Ok(directory) => {
                    ui.small(format!("Plugin scripts: {}", directory.display()));
                    ui.separator();

                    match mode {
                        ScriptDialog::Load => match script_files::list() {
                            Ok(files) => {
                                ScrollArea::vertical()
                                    .max_height(220.0)
                                    .show(ui, |ui| {
                                        if files.is_empty() {
                                            ui.label("No .koto files yet. Use Save to create one.");
                                        }
                                        for path in files {
                                            let name = path
                                                .file_name()
                                                .map(|name| name.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| "Unnamed.koto".to_owned());
                                            if ui.selectable_label(false, &name).clicked() {
                                                match script_files::load(&path) {
                                                    Ok(contents) => {
                                                        let index = params.selected_snippet_index();
                                                        match params.snippets.write() {
                                                            Ok(mut snippets) => {
                                                                if let Some(snippet) =
                                                                    snippets.get_mut(index)
                                                                {
                                                                    snippet.code =
                                                                        contents.clone();
                                                                    state.text_buffer = contents;
                                                                    state.file_notice =
                                                                        format!("Loaded {name}");
                                                                    close_after_action = true;
                                                                } else {
                                                                    state.file_notice =
                                                                        "No selected snippet to load into."
                                                                            .to_owned();
                                                                }
                                                            }
                                                            Err(_) => {
                                                                state.file_notice =
                                                                    "Could not update the selected snippet."
                                                                        .to_owned();
                                                            }
                                                        }
                                                    }
                                                    Err(error) => state.file_notice = error,
                                                }
                                            }
                                        }
                                    });
                            }
                            Err(error) => state.file_notice = error,
                        },
                        ScriptDialog::Save => {
                            ui.label("File name:");
                            ui.text_edit_singleline(&mut state.file_name);
                            if ui.button("Save").clicked() {
                                match script_files::save(
                                    &state.file_name,
                                    &state.text_buffer,
                                ) {
                                    Ok(path) => {
                                        let name = path
                                            .file_name()
                                            .map(|name| name.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "script.koto".to_owned());
                                        state.file_notice = format!("Saved {name}");
                                        close_after_action = true;
                                    }
                                    Err(error) => state.file_notice = error,
                                }
                            }
                        }
                    }
                }
                Err(error) => state.file_notice = error,
            }

            if !state.file_notice.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &state.file_notice);
            }
            if ui.button("Cancel").clicked() {
                close_after_action = true;
            }
        });

    if close_after_action || !open {
        state.file_dialog = None;
    }
}

fn koto_syntax() -> Syntax {
    Syntax::new("koto")
        .with_case_sensitive(true)
        .with_comment("#")
        .with_comment_multiline(["#-", "-#"])
        .with_hyperlinks(["http"])
        .with_keywords([
            "as",
            "assert",
            "assert_eq",
            "assert_ne",
            "assert_near",
            "break",
            "catch",
            "continue",
            "debug",
            "else",
            "export",
            "finally",
            "for",
            "from",
            "if",
            "import",
            "in",
            "loop",
            "match",
            "return",
            "switch",
            "then",
            "throw",
            "try",
            "until",
            "while",
            "yield",
        ])
        .with_special(["false", "null", "self", "true"])
}

#[derive(Default)]
struct GuiState {
    console: Vec<PipeMessage>,
    // to prevent locks, we clone the whole snippet code, when the selected snippet is changed. then
    // we update the snippet in params only when this value is update
    text_buffer: String,
    file_dialog: Option<ScriptDialog>,
    file_name: String,
    file_notice: String,
}
