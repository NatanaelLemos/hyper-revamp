//! Settings window: a separate viewport with a visual editor (Profiles,
//! Themes, App, Terminal, Advanced, Keymaps) and a raw JSON tab.
//!
//! Edits mutate a draft of the *user's* config document — never the
//! defaults-merged view the rest of the app reads. Saving a merged document
//! would bake every default into the user's file, so that the next release's
//! changed defaults could no longer reach them. Fields therefore fall back to
//! the defaults only for *display*, and each field the user's file actually
//! declares gets a reset control that removes the key again.

use egui::{Color32, ComboBox, Context, RichText, ScrollArea, TextEdit, Ui, ViewportBuilder, ViewportId};
use hyper_config::{ConfigSnapshot, ThemeColors};
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Profiles,
    Themes,
    App,
    Terminal,
    Advanced,
    Keymaps,
    Json,
}

#[derive(Clone, Copy, PartialEq)]
enum SshTestStatus {
    Idle,
    Testing,
    Ok,
    Failed,
}

pub struct SettingsWindow {
    pub open: bool,
    section: Section,
    /// Draft of the user's document — the thing that gets written back.
    doc: Value,
    /// The user's document as last read from (or written to) disk. `dirty` is
    /// derived by comparing against this rather than tracked by hand, so a
    /// change that is later undone stops counting as unsaved.
    saved_doc: Value,
    /// Shipped defaults, for display fallback only. Never merged into `doc`.
    defaults: Value,
    json_text: String,
    saved_text: String,
    json_error: Option<String>,
    /// The file changed underneath an unsaved draft.
    reload_requested: bool,
    selected_profile: usize,
    selected_theme: Option<String>,
    loaded_generation: u64,
    profile_overrides_text: Option<(usize, String, bool)>, // (profile idx, text, valid)
    ssh_test: SshTestStatus,
    ssh_test_detail: String,
    ssh_test_rx: Option<crossbeam_channel::Receiver<Result<(), String>>>,
    new_env_key: String,
    new_keymap_cmd: String,
    status: Option<String>,
}

impl Default for SettingsWindow {
    fn default() -> Self {
        SettingsWindow {
            open: false,
            section: Section::Profiles,
            doc: Value::Object(Map::new()),
            saved_doc: Value::Object(Map::new()),
            defaults: serde_json::from_str(hyper_config::DEFAULT_CONFIG_JSON)
                .unwrap_or_else(|_| Value::Object(Map::new())),
            json_text: String::new(),
            saved_text: String::new(),
            json_error: None,
            reload_requested: false,
            selected_profile: 0,
            selected_theme: None,
            loaded_generation: 0,
            profile_overrides_text: None,
            ssh_test: SshTestStatus::Idle,
            ssh_test_detail: String::new(),
            ssh_test_rx: None,
            new_env_key: String::new(),
            new_keymap_cmd: String::new(),
            status: None,
        }
    }
}

fn obj<'a>(value: &'a mut Value, key: &str) -> &'a mut Map<String, Value> {
    if !value.get(key).is_some_and(Value::is_object) {
        value[key] = Value::Object(Map::new());
    }
    value[key].as_object_mut().unwrap()
}

/// `JSON.stringify(value, null, 2) + '\n'`.
fn format_json(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.push('\n');
    text
}

/// Line-ending and trailing-whitespace insensitive, so that a file saved by
/// another editor doesn't read as an unsaved change.
fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_string()
}

fn round_to(value: f64, places: u32) -> f64 {
    let factor = 10f64.powi(places as i32);
    (value * factor).round() / factor
}

impl SettingsWindow {
    pub fn open_window(&mut self, config: &ConfigSnapshot) {
        self.open = true;
        if !self.dirty() || self.loaded_generation != config.generation {
            self.load_from(config);
        }
        // Debug affordance: jump to a section by name.
        if let Ok(section) = std::env::var("HYPER_OPEN_SETTINGS") {
            self.section = match section.as_str() {
                "themes" => Section::Themes,
                "app" => Section::App,
                "terminal" => Section::Terminal,
                "advanced" => Section::Advanced,
                "keymaps" => Section::Keymaps,
                "json" => Section::Json,
                _ => Section::Profiles,
            };
        }
    }

    fn load_from(&mut self, config: &ConfigSnapshot) {
        self.doc = config.user_document.clone();
        self.saved_doc = self.doc.clone();
        // The raw editor shows the file's own bytes — comments aren't legal
        // JSON so there are none to lose, but key order and formatting are the
        // user's and a re-serialization would quietly rewrite both.
        self.json_text = config
            .user_text
            .clone()
            .unwrap_or_else(|| format_json(&self.doc));
        self.saved_text = self.json_text.clone();
        self.json_error = None;
        self.reload_requested = false;
        self.loaded_generation = config.generation;
        self.profile_overrides_text = None;
        self.status = None;
    }

    fn visual_dirty(&self) -> bool {
        self.doc != self.saved_doc
    }

    fn text_dirty(&self) -> bool {
        normalize_text(&self.json_text) != normalize_text(&self.saved_text)
    }

    /// Only the active editor's draft counts: the other one is re-synced from
    /// it on switch, so it is never the thing the user would lose.
    fn dirty(&self) -> bool {
        if self.section == Section::Json {
            self.text_dirty()
        } else {
            self.visual_dirty()
        }
    }

    /// A draft changed; clear any stale "Saved" note.
    fn touch(&mut self) {
        self.status = None;
    }

    /// Move between the visual editor and the raw JSON tab, carrying the draft
    /// across. Switching *out of* an unparseable JSON draft is refused rather
    /// than silently discarding it.
    fn switch_to(&mut self, section: Section) {
        if section == self.section {
            return;
        }
        if section == Section::Json {
            self.json_text = format_json(&self.doc);
            self.json_error = None;
            self.section = section;
            return;
        }
        if self.section == Section::Json {
            match serde_json::from_str::<Value>(&self.json_text) {
                Ok(parsed) if parsed.is_object() => {
                    self.doc = parsed;
                    self.json_error = None;
                }
                Ok(_) => {
                    self.json_error = Some("Configuration root must be a JSON object.".into());
                    return;
                }
                Err(err) => {
                    self.json_error = Some(format!(
                        "Fix JSON errors before switching back to the visual editor: {err}"
                    ));
                    return;
                }
            }
        }
        self.section = section;
    }

    fn save(&mut self) {
        // Both tabs save through the same text path, so the JSON tab's literal
        // bytes reach disk and the visual tab still gets validated.
        let source: Result<(String, Value), String> = if self.section == Section::Json {
            match serde_json::from_str::<Value>(&self.json_text) {
                Ok(doc) => Ok((self.json_text.clone(), doc)),
                Err(err) => Err(err.to_string()),
            }
        } else {
            Ok((format_json(&self.doc), self.doc.clone()))
        };

        let result = source.and_then(|(text, doc)| {
            hyper_config::save_text(&text).map(|()| (text, doc))
        });

        match result {
            Ok((text, doc)) => {
                self.doc = doc;
                self.saved_doc = self.doc.clone();
                self.json_text = text;
                self.saved_text = self.json_text.clone();
                self.json_error = None;
                self.reload_requested = false;
                self.status = Some(format!("Saved {}", hyper_config::paths::cfg_path().display()));
            }
            Err(err) => {
                self.json_error = Some(err.clone());
                self.status = Some(format!("Save failed: {err}"));
            }
        }
    }

    /// Render as a separate OS window. Call every frame from the app.
    pub fn show(&mut self, ctx: &Context, config: &ConfigSnapshot) {
        if !self.open {
            return;
        }

        // Pick up external edits. With unsaved work in hand, say so instead of
        // either clobbering the file on the next save or throwing the draft
        // away.
        if self.loaded_generation != config.generation {
            if self.dirty() {
                self.reload_requested = true;
            } else {
                self.load_from(config);
            }
        }

        // Poll a running ssh test.
        if let Some(rx) = &self.ssh_test_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(()) => {
                        self.ssh_test = SshTestStatus::Ok;
                        self.ssh_test_detail = "Connection OK".into();
                    }
                    Err(err) => {
                        self.ssh_test = SshTestStatus::Failed;
                        self.ssh_test_detail = err;
                    }
                }
                self.ssh_test_rx = None;
            }
        }

        let mut open = self.open;
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("hyper-settings"),
            ViewportBuilder::default()
                .with_title("Hyper Revamp Settings")
                .with_inner_size([900.0, 680.0])
                .with_min_inner_size([760.0, 480.0]),
            |ctx, _class| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.window_ui(ui, config);
                });
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
            },
        );
        self.open = open;
    }

    fn window_ui(&mut self, ui: &mut Ui, config: &ConfigSnapshot) {
        // Top bar: section nav + save.
        ui.horizontal(|ui| {
            ui.heading("Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = self.dirty();
                let save_label = if dirty { "Save •" } else { "Save" };
                let can_save = dirty
                    && (self.section != Section::Json
                        || serde_json::from_str::<Value>(&self.json_text).is_ok());
                if ui
                    .add_enabled(can_save, egui::Button::new(save_label))
                    .clicked()
                {
                    self.save();
                }
                if ui.button("Reload").clicked() {
                    self.load_from(config);
                }
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).weak());
                }
            });
        });

        if self.reload_requested {
            ui.horizontal(|ui| {
                ui.colored_label(
                    Color32::from_rgb(0xe6, 0xa2, 0x3c),
                    "Configuration changed on disk. Saving now overwrites those edits.",
                );
                if ui.button("Discard mine and reload").clicked() {
                    self.load_from(config);
                }
            });
        }
        if let Some(err) = &self.json_error {
            ui.colored_label(Color32::from_rgb(0xff, 0x6b, 0x6b), err);
        }
        ui.separator();

        ui.horizontal_top(|ui| {
            // Left nav.
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for (section, label) in [
                    (Section::Profiles, "Profiles"),
                    (Section::Themes, "Themes"),
                    (Section::App, "App"),
                    (Section::Terminal, "Terminal"),
                    (Section::Advanced, "Advanced"),
                    (Section::Keymaps, "Keymaps"),
                    (Section::Json, "JSON editor"),
                ] {
                    if ui
                        .selectable_label(self.section == section, label)
                        .clicked()
                    {
                        self.switch_to(section);
                    }
                }
            });
            ui.separator();

            ui.vertical(|ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.section {
                        Section::Profiles => self.profiles_ui(ui, config),
                        Section::Themes => self.themes_ui(ui, config),
                        Section::App => self.app_ui(ui, config),
                        Section::Terminal => self.terminal_ui(ui),
                        Section::Advanced => self.advanced_ui(ui),
                        Section::Keymaps => self.keymaps_ui(ui),
                        Section::Json => self.json_ui(ui),
                    }
                });
            });
        });
    }

    // ---- helpers on the draft document ----

    /// What this field is currently worth: the user's value if their file has
    /// one, otherwise the shipped default (shown, never written).
    fn effective(&self, path: &[&str]) -> Option<&Value> {
        get_path(&self.doc, path).or_else(|| get_path(&self.defaults, path))
    }

    /// Reset control, shown only for keys the user's file actually declares
    /// (the original's `canReset = hasOwnPath(draft, path)`). It unsets the
    /// key so the default applies again — writing the default value back in
    /// its place would pin it forever.
    fn reset_button(&mut self, ui: &mut Ui, path: &[&str]) {
        if get_path(&self.doc, path).is_none() {
            ui.add_space(20.0);
            return;
        }
        if ui
            .small_button("↺")
            .on_hover_text("Reset to default (removes this key from your config)")
            .clicked()
        {
            remove_path(&mut self.doc, path);
            self.touch();
        }
    }

    fn edit_string(&mut self, ui: &mut Ui, label: &str, path: &[&str], hint: &str) {
        let mut current = self
            .effective(path)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(label);
            let resp = ui.add(
                TextEdit::singleline(&mut current)
                    .hint_text(hint)
                    .desired_width(260.0),
            );
            if resp.changed() {
                set_path(&mut self.doc, path, Value::String(current.clone()));
                self.touch();
            }
            self.reset_button(ui, path);
        });
    }

    fn edit_bool(&mut self, ui: &mut Ui, label: &str, path: &[&str], default: bool) {
        let mut current = self.effective(path).and_then(Value::as_bool).unwrap_or(default);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut current, label).changed() {
                set_path(&mut self.doc, path, Value::Bool(current));
                self.touch();
            }
            self.reset_button(ui, path);
        });
    }

    fn edit_number(
        &mut self,
        ui: &mut Ui,
        label: &str,
        path: &[&str],
        default: f64,
        int: bool,
        range: std::ops::RangeInclusive<f64>,
    ) {
        let mut current = self.effective(path).and_then(Value::as_f64).unwrap_or(default);
        ui.horizontal(|ui| {
            ui.label(label);
            let speed = if int { 1.0 } else { 0.05 };
            if ui
                .add(
                    egui::DragValue::new(&mut current)
                        .speed(speed)
                        .range(range.clone()),
                )
                .changed()
            {
                let value = if int {
                    json!(current.round() as i64)
                } else {
                    // A drag accumulates in binary floating point; without this
                    // a line height lands in the file as 1.0500000000000003.
                    json!(round_to(current, 4))
                };
                set_path(&mut self.doc, path, value);
                self.touch();
            }
            self.reset_button(ui, path);
        });
    }

    fn edit_color(&mut self, ui: &mut Ui, label: &str, path: &[&str], default: &str) {
        let current = self
            .effective(path)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| default.to_string());
        let rgba = hyper_config::parse_color(&current).unwrap_or([0, 0, 0, 255]);
        let mut color = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
        ui.horizontal(|ui| {
            ui.label(label);
            if ui.color_edit_button_srgba(&mut color).changed() {
                let hex = if color.a() == 255 {
                    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
                } else {
                    format!(
                        "rgba({},{},{},{:.2})",
                        color.r(),
                        color.g(),
                        color.b(),
                        color.a() as f32 / 255.0
                    )
                };
                set_path(&mut self.doc, path, Value::String(hex));
                self.touch();
            }
            ui.label(RichText::new(current).weak().small());
            self.reset_button(ui, path);
        });
    }

    // ---- sections ----

    /// The effective profile list (user's array, else the shipped one).
    fn profiles_view(&self) -> Vec<Value> {
        self.effective(&["config", "profiles"])
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![json!({"name": "default", "config": {}})])
    }

    /// Materialize the effective profile list into the draft so an element can
    /// be added or removed. Called only from real edits: doing it on render
    /// would mark the config unsaved just for opening the tab.
    fn profiles_draft(&mut self) -> &mut Vec<Value> {
        if !get_path(&self.doc, &["config", "profiles"]).is_some_and(Value::is_array) {
            let seed = Value::Array(self.profiles_view());
            set_path(&mut self.doc, &["config", "profiles"], seed);
        }
        obj(&mut self.doc, "config")
            .get_mut("profiles")
            .unwrap()
            .as_array_mut()
            .unwrap()
    }

    fn profiles_ui(&mut self, ui: &mut Ui, config: &ConfigSnapshot) {
        ui.horizontal(|ui| {
            ui.strong("Profiles");
            if ui.button("+ Add").clicked() {
                let n = self.profiles_draft().len();
                self.profiles_draft()
                    .push(json!({"name": format!("profile-{}", n + 1), "config": {}}));
                self.selected_profile = n;
                self.touch();
            }
        });
        ui.add_space(4.0);

        let names: Vec<String> = self
            .profiles_view()
            .iter()
            .map(|p| {
                p.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unnamed")
                    .to_string()
            })
            .collect();
        let default_profile = self
            .effective(&["config", "defaultProfile"])
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "default".into());

        ui.horizontal_top(|ui| {
            // Profile list.
            ui.vertical(|ui| {
                ui.set_width(160.0);
                for (i, name) in names.iter().enumerate() {
                    let label = if *name == default_profile {
                        format!("{name} ★")
                    } else {
                        name.clone()
                    };
                    if ui
                        .selectable_label(self.selected_profile == i, label)
                        .clicked()
                    {
                        self.selected_profile = i;
                        self.profile_overrides_text = None;
                        self.ssh_test = SshTestStatus::Idle;
                    }
                }
            });
            ui.separator();

            // Profile detail.
            ui.vertical(|ui| {
                if names.is_empty() {
                    ui.label("No profiles.");
                    return;
                }
                let idx = self.selected_profile.min(names.len() - 1);
                self.selected_profile = idx;
                let idx_str = idx.to_string();

                // Name.
                let name_path = ["config", "profiles", idx_str.as_str(), "name"];
                self.edit_string(ui, "Name", &name_path, "profile name");

                // Default toggle.
                let this_name = self
                    .effective(&name_path)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                let is_default = this_name == default_profile;
                ui.horizontal(|ui| {
                    if is_default {
                        ui.label(RichText::new("Default profile ★").strong());
                    } else if ui.button("Set as default").clicked() {
                        set_path(
                            &mut self.doc,
                            &["config", "defaultProfile"],
                            Value::String(this_name.clone()),
                        );
                        self.touch();
                    }
                    if names.len() > 1 && ui.button("Delete profile").clicked() {
                        self.profiles_draft().remove(idx);
                        self.selected_profile = 0;
                        self.profile_overrides_text = None;
                        self.touch();
                    }
                });
                ui.separator();

                // Type toggle.
                let type_path = ["config", "profiles", idx_str.as_str(), "type"];
                let current_type = self
                    .effective(&type_path)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "local".into());
                let mut is_ssh = current_type == "ssh";
                ui.horizontal(|ui| {
                    ui.label("Type");
                    if ui.selectable_label(!is_ssh, "Local").clicked() {
                        is_ssh = false;
                        set_path(&mut self.doc, &type_path, Value::String("local".into()));
                        self.touch();
                    }
                    if ui.selectable_label(is_ssh, "SSH").clicked() {
                        is_ssh = true;
                        set_path(&mut self.doc, &type_path, Value::String("ssh".into()));
                        self.touch();
                    }
                });

                if is_ssh {
                    ui.group(|ui| {
                        ui.strong("SSH connection");
                        let base = ["config", "profiles", idx_str.as_str(), "ssh"];
                        let field = |f: &'static str| {
                            let mut path = base.to_vec();
                            path.push(f);
                            path
                        };
                        self.edit_string(ui, "Host", &field("host"), "example.com");
                        self.edit_string(ui, "User", &field("user"), "username");
                        self.edit_number(ui, "Port", &field("port"), 22.0, true, 1.0..=65535.0);

                        let auth_path = field("authType");
                        let auth = self
                            .effective(&auth_path)
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_else(|| "publickey".into());
                        ui.horizontal(|ui| {
                            ui.label("Auth");
                            if ui.selectable_label(auth == "publickey", "Public key").clicked() {
                                set_path(&mut self.doc, &auth_path, json!("publickey"));
                                self.touch();
                            }
                            if ui.selectable_label(auth == "password", "Password").clicked() {
                                set_path(&mut self.doc, &auth_path, json!("password"));
                                self.touch();
                            }
                        });
                        if auth == "password" {
                            let pw_path = field("password");
                            let mut pw = self
                                .effective(&pw_path)
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("Password");
                                if ui
                                    .add(TextEdit::singleline(&mut pw).password(true).desired_width(200.0))
                                    .changed()
                                {
                                    set_path(&mut self.doc, &pw_path, Value::String(pw.clone()));
                                    self.touch();
                                }
                                self.reset_button(ui, &pw_path);
                            });
                            ui.label(
                                RichText::new("Stored in plaintext in the config file.")
                                    .weak()
                                    .small(),
                            );
                        } else {
                            self.edit_string(
                                ui,
                                "Identity file",
                                &field("identityFile"),
                                "~/.ssh/id_ed25519",
                            );
                            self.edit_bool(ui, "Forward agent (-A)", &field("forwardAgent"), false);
                        }

                        ui.horizontal(|ui| {
                            let testing = self.ssh_test == SshTestStatus::Testing;
                            if ui
                                .add_enabled(!testing, egui::Button::new("Test connection"))
                                .clicked()
                            {
                                self.start_ssh_test(idx);
                            }
                            match self.ssh_test {
                                SshTestStatus::Idle => {}
                                SshTestStatus::Testing => {
                                    ui.spinner();
                                }
                                SshTestStatus::Ok => {
                                    ui.colored_label(Color32::from_rgb(0x2e, 0xcc, 0x71), "✓ OK");
                                }
                                SshTestStatus::Failed => {
                                    ui.colored_label(
                                        Color32::from_rgb(0xff, 0x6b, 0x6b),
                                        RichText::new(format!("✗ {}", self.ssh_test_detail))
                                            .small(),
                                    );
                                }
                            }
                        });
                    });
                }

                // Theme + accent.
                ui.separator();
                let theme_path = ["config", "profiles", idx_str.as_str(), "theme"];
                let current_theme = self
                    .effective(&theme_path)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                // The whole registry, not just the bundled set: a theme the
                // user declared under `config.themes` is assignable here too.
                let theme_ids: Vec<String> = config.themes.keys().cloned().collect();
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    let display = if current_theme.is_empty() {
                        "(default theme)".to_string()
                    } else {
                        current_theme.clone()
                    };
                    ComboBox::from_id_salt("profile-theme")
                        .selected_text(display)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current_theme.is_empty(), "(default theme)").clicked() {
                                remove_path(&mut self.doc, &theme_path);
                                self.touch();
                            }
                            for id in &theme_ids {
                                if ui.selectable_label(current_theme == *id, id).clicked() {
                                    set_path(&mut self.doc, &theme_path, json!(id));
                                    self.touch();
                                }
                            }
                        });
                });
                let color_path = ["config", "profiles", idx_str.as_str(), "color"];
                self.edit_color(ui, "Tab accent", &color_path, "#50e3c2");

                // Common overrides + raw JSON for the rest.
                ui.separator();
                ui.strong("Overrides");
                let over = |f: &'static str| ["config", "profiles", idx_str.as_str(), "config", f];
                self.edit_string(ui, "Shell", &over("shell"), "(system default)");
                self.edit_string(ui, "Working directory", &over("workingDirectory"), "~");

                ui.collapsing("All overrides (JSON)", |ui| {
                    let needs_load = self
                        .profile_overrides_text
                        .as_ref()
                        .is_none_or(|(i, _, _)| *i != idx);
                    if needs_load {
                        let text = get_path(&self.doc, &["config", "profiles", &idx_str, "config"])
                            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                            .unwrap_or_else(|| "{}".into());
                        self.profile_overrides_text = Some((idx, text, true));
                    }
                    if let Some((_, text, valid)) = &mut self.profile_overrides_text {
                        let response = ui.add(
                            TextEdit::multiline(text)
                                .code_editor()
                                .desired_rows(8)
                                .desired_width(f32::INFINITY),
                        );
                        if response.changed() {
                            match serde_json::from_str::<Value>(text) {
                                Ok(parsed) if parsed.is_object() => {
                                    *valid = true;
                                    set_path(
                                        &mut self.doc,
                                        &["config", "profiles", &idx_str, "config"],
                                        parsed,
                                    );
                                    self.status = None;
                                }
                                _ => *valid = false,
                            }
                        }
                        if !*valid {
                            ui.colored_label(
                                Color32::from_rgb(0xff, 0x6b, 0x6b),
                                "Invalid JSON — not applied",
                            );
                        }
                    }
                });
            });
        });
    }

    fn start_ssh_test(&mut self, profile_idx: usize) {
        let idx = profile_idx.to_string();
        let get = |f: &str| {
            self.effective(&["config", "profiles", &idx, "ssh", f])
                .and_then(|v| v.as_str().map(str::to_string))
        };
        let host = get("host").unwrap_or_default();
        let user = get("user").unwrap_or_default();
        if host.is_empty() || user.is_empty() {
            self.ssh_test = SshTestStatus::Failed;
            self.ssh_test_detail = "host and user are required".into();
            return;
        }
        let port = self
            .effective(&["config", "profiles", &idx, "ssh", "port"])
            .and_then(Value::as_u64);
        let identity = get("identityFile");
        let password_auth = get("authType").as_deref() == Some("password");
        let password = get("password");

        let (tx, rx) = crossbeam_channel::bounded(1);
        self.ssh_test = SshTestStatus::Testing;
        self.ssh_test_rx = Some(rx);

        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("ssh");
            cmd.arg("-o").arg("ConnectTimeout=8");
            cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
            if let Some(port) = port {
                cmd.arg("-p").arg(port.to_string());
            }
            // Deleted once ssh has exited, whatever the outcome — the password
            // sits in this file in the clear until then.
            let mut password_file = None;
            if password_auth {
                cmd.arg("-o").arg("PreferredAuthentications=password");
                cmd.arg("-o").arg("PubkeyAuthentication=no");
                // Reuse the app's askpass helper mode for the test.
                if let (Some(password), Ok(exe)) = (password, std::env::current_exe()) {
                    match hyper_term::ssh::write_password_file(&password) {
                        Ok(path) => {
                            cmd.env("SSH_ASKPASS", exe);
                            cmd.env("SSH_ASKPASS_REQUIRE", "force");
                            cmd.env("DISPLAY", ":0");
                            cmd.env("HYPER_ASKPASS_FILE", &path);
                            password_file = Some(path);
                        }
                        Err(err) => {
                            let _ = tx.send(Err(err.to_string()));
                            return;
                        }
                    }
                }
            } else {
                cmd.arg("-o").arg("BatchMode=yes");
                if let Some(identity) = identity.filter(|s| !s.is_empty()) {
                    cmd.arg("-i").arg(identity);
                }
            }
            cmd.arg(format!("{user}@{host}")).arg("exit");
            cmd.stdin(std::process::Stdio::null());

            let result = match cmd.output() {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(stderr.lines().last().unwrap_or("connection failed").to_string())
                }
                Err(err) => Err(err.to_string()),
            };
            if let Some(path) = password_file {
                let _ = std::fs::remove_file(path);
            }
            let _ = tx.send(result);
        });
    }

    fn themes_ui(&mut self, ui: &mut Ui, config: &ConfigSnapshot) {
        let default_theme = self
            .effective(&["config", "defaultTheme"])
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "default".into());

        ui.horizontal(|ui| {
            ui.strong("Themes");
            ui.label(
                RichText::new("Click a card to set the default theme; themes can also be assigned per profile.")
                    .weak()
                    .small(),
            );
        });
        ui.add_space(6.0);

        let themes: Vec<(String, ThemeColors)> = config
            .themes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let card_w = 190.0;
        let avail = ui.available_width();
        let per_row = ((avail / (card_w + 10.0)) as usize).max(1);

        for chunk in themes.chunks(per_row) {
            ui.horizontal(|ui| {
                for (id, theme) in chunk {
                    let selected = *id == default_theme;
                    let (rect, response) = ui.allocate_exact_size(
                        egui::Vec2::new(card_w, 84.0),
                        egui::Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    let bg = theme
                        .background_color
                        .as_deref()
                        .and_then(hyper_config::parse_color)
                        .map(|[r, g, b, _]| Color32::from_rgb(r, g, b))
                        .unwrap_or(Color32::BLACK);
                    let fg = theme
                        .foreground_color
                        .as_deref()
                        .and_then(hyper_config::parse_color)
                        .map(|[r, g, b, _]| Color32::from_rgb(r, g, b))
                        .unwrap_or(Color32::WHITE);
                    painter.rect_filled(rect, 6.0, bg);
                    painter.rect_stroke(
                        rect,
                        6.0,
                        egui::Stroke::new(
                            if selected { 2.0 } else { 1.0 },
                            if selected {
                                Color32::from_rgb(0x4a, 0x90, 0xd9)
                            } else {
                                Color32::from_gray(70)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        rect.min + egui::Vec2::new(10.0, 14.0),
                        egui::Align2::LEFT_CENTER,
                        id,
                        egui::FontId::proportional(13.0),
                        fg,
                    );
                    painter.text(
                        rect.min + egui::Vec2::new(10.0, 34.0),
                        egui::Align2::LEFT_CENTER,
                        "$ echo hello",
                        egui::FontId::monospace(11.0),
                        fg,
                    );
                    // ANSI swatch strip.
                    if let Some(colors) = &theme.colors {
                        for (i, c) in colors.as_array().iter().take(8).enumerate() {
                            if let Some(c) = c {
                                if let Some([r, g, b, _]) = hyper_config::parse_color(c) {
                                    painter.rect_filled(
                                        egui::Rect::from_min_size(
                                            rect.min
                                                + egui::Vec2::new(
                                                    10.0 + i as f32 * 20.0,
                                                    54.0,
                                                ),
                                            egui::Vec2::new(16.0, 16.0),
                                        ),
                                        3.0,
                                        Color32::from_rgb(r, g, b),
                                    );
                                }
                            }
                        }
                    }
                    if response.clicked() {
                        set_path(&mut self.doc, &["config", "defaultTheme"], json!(id));
                        self.touch();
                    }
                }
            });
            ui.add_space(8.0);
        }

        ui.separator();
        ui.collapsing("Edit theme colors (config.themes overrides)", |ui| {
            ui.label(
                RichText::new(
                    "Overrides declared in config.themes win over bundled themes with the same id.",
                )
                .weak()
                .small(),
            );
            let ids: Vec<String> = themes.iter().map(|(id, _)| id.clone()).collect();
            let selected = self
                .selected_theme
                .clone()
                .unwrap_or_else(|| default_theme.clone());
            ComboBox::from_id_salt("theme-edit")
                .selected_text(selected.clone())
                .show_ui(ui, |ui| {
                    for id in &ids {
                        if ui.selectable_label(selected == *id, id).clicked() {
                            self.selected_theme = Some(id.clone());
                        }
                    }
                });
            let theme_id = self.selected_theme.clone().unwrap_or(selected);

            // Seed the override object from the effective theme on first edit.
            let base_theme = config.themes.get(&theme_id).cloned().unwrap_or_default();
            let seed = serde_json::to_value(&base_theme).unwrap_or(json!({}));
            let path = ["config", "themes", theme_id.as_str()];
            if get_path(&self.doc, &path).is_none() {
                if ui.button("Customize this theme").clicked() {
                    set_path(&mut self.doc, &path, seed);
                    self.touch();
                }
                return;
            }
            if ui.button("Stop customizing (remove override)").clicked() {
                remove_path(&mut self.doc, &path);
                self.touch();
                return;
            }

            for (label, key, default) in [
                ("Background", "backgroundColor", "#000"),
                ("Foreground", "foregroundColor", "#fff"),
                ("Cursor", "cursorColor", "#fff"),
                ("Selection", "selectionColor", "rgba(248,28,229,0.3)"),
                ("Border", "borderColor", "#333"),
            ] {
                let p = ["config", "themes", theme_id.as_str(), key];
                self.edit_color(ui, label, &p, default);
            }
            ui.label(RichText::new("ANSI palette").strong());
            for key in [
                "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
                "lightBlack", "lightRed", "lightGreen", "lightYellow", "lightBlue",
                "lightMagenta", "lightCyan", "lightWhite",
            ] {
                let p = ["config", "themes", theme_id.as_str(), "colors", key];
                self.edit_color(ui, key, &p, "#000000");
            }
        });
    }

    fn app_ui(&mut self, ui: &mut Ui, config: &ConfigSnapshot) {
        ui.strong("Application");
        ui.add_space(4.0);
        self.edit_bool(
            ui,
            "Quit when the last window closes",
            &["config", "quitOnLastWindowClosed"],
            true,
        );
        self.edit_bool(
            ui,
            "Register as default ssh:// handler",
            &["config", "defaultSSHApp"],
            true,
        );
        self.edit_bool(
            ui,
            "Restore session (tabs) on launch",
            &["config", "restoreSession"],
            false,
        );

        ui.add_space(6.0);
        let profiles: Vec<String> = config
            .root
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let current = self
            .effective(&["config", "defaultProfile"])
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "default".into());
        ui.horizontal(|ui| {
            ui.label("Default profile");
            ComboBox::from_id_salt("default-profile")
                .selected_text(current.clone())
                .show_ui(ui, |ui| {
                    for name in &profiles {
                        if ui.selectable_label(current == *name, name).clicked() {
                            set_path(&mut self.doc, &["config", "defaultProfile"], json!(name));
                            self.touch();
                        }
                    }
                });
            self.reset_button(ui, &["config", "defaultProfile"]);
        });

        ui.add_space(8.0);
        ui.label(
            RichText::new("Auto-update and plugins from the Electron app are not part of the native build; their config keys are preserved on save.")
                .weak()
                .small(),
        );
    }

    /// Opacity is either a bare number or `{focus, blur}`. Editing one mode
    /// must not silently drop the other's value on the floor, and the range is
    /// the full 0..1 the app clamps to — not a UI-invented floor.
    fn edit_opacity(&mut self, ui: &mut Ui, label: &str, path: &[&str], default: f64) {
        let mut value = self.effective(path).and_then(Value::as_f64).unwrap_or(default) as f32;
        ui.horizontal(|ui| {
            ui.label(label);
            if ui
                .add(egui::Slider::new(&mut value, 0.0..=1.0).step_by(0.01))
                .changed()
            {
                set_path(&mut self.doc, path, json!(round_to(value as f64, 2)));
                self.touch();
            }
            self.reset_button(ui, path);
        });
    }

    fn terminal_ui(&mut self, ui: &mut Ui) {
        ui.strong("Terminal");
        ui.add_space(4.0);

        // Cursor shape.
        let shape = self
            .effective(&["config", "cursorShape"])
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "BLOCK".into());
        ui.horizontal(|ui| {
            ui.label("Cursor shape");
            for option in ["BLOCK", "BEAM", "UNDERLINE"] {
                if ui.selectable_label(shape == option, option).clicked() {
                    set_path(&mut self.doc, &["config", "cursorShape"], json!(option));
                    self.touch();
                }
            }
            self.reset_button(ui, &["config", "cursorShape"]);
        });
        self.edit_bool(ui, "Cursor blink", &["config", "cursorBlink"], false);
        self.edit_color(
            ui,
            "Cursor color",
            &["config", "cursorColor"],
            "rgba(248,28,229,0.8)",
        );

        ui.separator();
        self.edit_number(ui, "Font size", &["config", "fontSize"], 14.0, true, 1.0..=200.0);
        self.edit_number(ui, "Line height", &["config", "lineHeight"], 1.0, false, 0.5..=5.0);
        self.edit_number(
            ui,
            "Letter spacing",
            &["config", "letterSpacing"],
            0.0,
            false,
            -5.0..=20.0,
        );
        self.edit_string(ui, "Padding (CSS)", &["config", "padding"], "12px 14px");
        self.edit_number(
            ui,
            "Scrollback lines",
            &["config", "scrollback"],
            1000.0,
            true,
            0.0..=1_000_000.0,
        );

        ui.separator();
        self.edit_bool(ui, "Copy on select", &["config", "copyOnSelect"], false);
        self.edit_bool(
            ui,
            "Quick edit (right-click copies/pastes)",
            &["config", "quickEdit"],
            false,
        );

        // Bell. `lib/index.tsx` compares `bell.toUpperCase() !== 'SOUND'`, so
        // a config saying "sound" rings — the checkbox has to agree, or it
        // reads as off and unchecking it does nothing.
        let bell_on = self
            .effective(&["config", "bell"])
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("SOUND"));
        let mut bell = bell_on;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut bell, "Bell").changed() {
                let value = if bell { json!("SOUND") } else { json!(false) };
                set_path(&mut self.doc, &["config", "bell"], value);
                self.touch();
            }
            self.reset_button(ui, &["config", "bell"]);
        });

        // Opacity: number | {focus, blur}.
        let opacity_path = ["config", "opacity"];
        let effective = self.effective(&opacity_path).cloned().unwrap_or(json!(1.0));
        let focus_blur = effective.is_object();
        ui.horizontal(|ui| {
            ui.label("Window opacity");
            if ui.selectable_label(!focus_blur, "Single").clicked() && focus_blur {
                set_path(&mut self.doc, &opacity_path, json!(1.0));
                self.touch();
            }
            if ui
                .selectable_label(focus_blur, "Focused / blurred")
                .clicked()
                && !focus_blur
            {
                set_path(
                    &mut self.doc,
                    &opacity_path,
                    json!({"focus": 1.0, "blur": 0.9}),
                );
                self.touch();
            }
            self.reset_button(ui, &opacity_path);
        });
        if focus_blur {
            self.edit_opacity(ui, "Focused", &["config", "opacity", "focus"], 1.0);
            self.edit_opacity(ui, "Blurred", &["config", "opacity", "blur"], 0.9);
        } else {
            self.edit_opacity(ui, "Opacity", &opacity_path, 1.0);
        }

        ui.separator();
        ui.collapsing("macOS modifiers", |ui| {
            self.edit_bool(
                ui,
                "Option is Meta (send ESC+key)",
                &["config", "modifierKeys", "altIsMeta"],
                false,
            );
        });
    }

    fn advanced_ui(&mut self, ui: &mut Ui) {
        ui.strong("Environment variables");
        ui.add_space(4.0);

        let env_entries: Vec<(String, String)> = self
            .effective(&["config", "env"])
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let mut remove_key = None;
        for (key, value) in &env_entries {
            let mut value = value.clone();
            ui.horizontal(|ui| {
                ui.monospace(key);
                if ui
                    .add(TextEdit::singleline(&mut value).desired_width(220.0))
                    .changed()
                {
                    set_path(&mut self.doc, &["config", "env", key], json!(value));
                    self.touch();
                }
                if ui.small_button("✕").clicked() {
                    remove_key = Some(key.clone());
                }
            });
        }
        if let Some(key) = remove_key {
            remove_path(&mut self.doc, &["config", "env", &key]);
            self.touch();
        }

        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.new_env_key)
                    .hint_text("NEW_VAR")
                    .desired_width(160.0),
            );
            if ui.button("+ Add").clicked() && !self.new_env_key.trim().is_empty() {
                let key = self.new_env_key.trim().to_string();
                set_path(&mut self.doc, &["config", "env", &key], json!(""));
                self.new_env_key.clear();
                self.touch();
            }
        });

        ui.separator();
        self.edit_string(
            ui,
            "Shell",
            &["config", "shell"],
            "(system default shell)",
        );
        ui.label(
            RichText::new("shellArgs and other list-valued options are editable in the JSON editor.")
                .weak()
                .small(),
        );
        ui.label(
            RichText::new("css / termCSS from the Electron app have no effect in the native build but are preserved in the file.")
                .weak()
                .small(),
        );
    }

    fn keymaps_ui(&mut self, ui: &mut Ui) {
        ui.strong("Keymaps");
        ui.label(
            RichText::new("Override bindings per command, e.g. tab:new → command+t. Separate several bindings for one command with commas.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);

        let entries: Vec<(String, Value)> = self
            .doc
            .get("keymaps")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let mut remove = None;
        for (cmd, value) in &entries {
            // A command may be bound to a list of chords. Rendering that list
            // as "a, b" and writing the joined string straight back would turn
            // two bindings into one unparseable chord.
            let was_array = value.is_array();
            let mut binding = match value {
                Value::String(s) => s.clone(),
                Value::Array(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => String::new(),
            };
            ui.horizontal(|ui| {
                ui.monospace(format!("{cmd:28}"));
                if ui
                    .add(TextEdit::singleline(&mut binding).desired_width(200.0))
                    .changed()
                {
                    let parts: Vec<&str> = binding
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                    let next = if was_array || parts.len() > 1 {
                        json!(parts)
                    } else {
                        json!(binding)
                    };
                    obj(&mut self.doc, "keymaps").insert(cmd.clone(), next);
                    self.touch();
                }
                if ui.small_button("✕").clicked() {
                    remove = Some(cmd.clone());
                }
            });
        }
        if let Some(cmd) = remove {
            obj(&mut self.doc, "keymaps").remove(&cmd);
            self.touch();
        }

        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.new_keymap_cmd)
                    .hint_text("tab:new")
                    .desired_width(200.0),
            );
            if ui.button("+ Add override").clicked() && !self.new_keymap_cmd.trim().is_empty() {
                let cmd = self.new_keymap_cmd.trim().to_string();
                obj(&mut self.doc, "keymaps").insert(cmd, json!(""));
                self.new_keymap_cmd.clear();
                self.touch();
            }
        });
    }

    fn json_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.strong("hyper-revamp.json");
            ui.label(
                RichText::new(hyper_config::paths::cfg_path().display().to_string())
                    .weak()
                    .small(),
            );
        });
        let response = ui.add_sized(
            ui.available_size(),
            TextEdit::multiline(&mut self.json_text)
                .code_editor()
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            self.touch();
            self.json_error = serde_json::from_str::<Value>(&self.json_text)
                .err()
                .map(|e| e.to_string());
        }
    }
}

// ---- JSON path helpers (ports of getAtPath/setAtPath/unsetAtPath) ----

fn get_path<'a>(doc: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = doc;
    for part in path {
        current = match current {
            Value::Object(map) => map.get(*part)?,
            Value::Array(arr) => arr.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn set_path(doc: &mut Value, path: &[&str], value: Value) {
    let mut current = doc;
    for (i, part) in path.iter().enumerate() {
        let last = i == path.len() - 1;
        let index = part.parse::<usize>().ok();

        if let Some(index) = index {
            if current.is_array() {
                let arr = current.as_array_mut().unwrap();
                while arr.len() <= index {
                    arr.push(Value::Null);
                }
                if last {
                    arr[index] = value;
                    return;
                }
                current = &mut arr[index];
                continue;
            }
        }

        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        let map = current.as_object_mut().unwrap();
        if last {
            map.insert((*part).to_string(), value);
            return;
        }
        // Create the missing container to match the *next* segment. Without
        // this, `profiles.0.name` on a config with no profiles builds an
        // object keyed "0" — which looks right in the editor and is invisible
        // to everything that reads the list.
        let child_is_index = path[i + 1].parse::<usize>().is_ok();
        let slot = map.entry((*part).to_string()).or_insert(Value::Null);
        if !slot.is_object() && !slot.is_array() {
            *slot = if child_is_index {
                Value::Array(Vec::new())
            } else {
                Value::Object(Map::new())
            };
        }
        current = slot;
    }
}

fn remove_path(doc: &mut Value, path: &[&str]) {
    if path.is_empty() {
        return;
    }
    let (last, parents) = path.split_last().unwrap();
    let mut current = doc;
    for part in parents {
        current = match current {
            Value::Object(map) => match map.get_mut(*part) {
                Some(v) => v,
                None => return,
            },
            Value::Array(arr) => match part.parse::<usize>().ok().and_then(|i| arr.get_mut(i)) {
                Some(v) => v,
                None => return,
            },
            _ => return,
        };
    }
    match current {
        Value::Object(map) => {
            map.remove(*last);
        }
        Value::Array(arr) => {
            if let Some(index) = last.parse::<usize>().ok().filter(|i| *i < arr.len()) {
                arr.remove(index);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers_roundtrip() {
        let mut doc = json!({"config": {"profiles": [{"name": "a"}]}});
        set_path(&mut doc, &["config", "profiles", "0", "name"], json!("b"));
        assert_eq!(
            get_path(&doc, &["config", "profiles", "0", "name"]),
            Some(&json!("b"))
        );
        set_path(&mut doc, &["config", "env", "FOO"], json!("1"));
        assert_eq!(doc["config"]["env"]["FOO"], "1");
        remove_path(&mut doc, &["config", "env", "FOO"]);
        assert!(get_path(&doc, &["config", "env", "FOO"]).is_none());
    }

    /// Writing through a numeric segment has to build an array. An object
    /// keyed "0" round-trips through this editor looking correct while the
    /// profile list silently reads as empty everywhere else.
    #[test]
    fn numeric_segment_creates_an_array_not_an_object() {
        let mut doc = json!({});
        set_path(&mut doc, &["config", "profiles", "0", "name"], json!("work"));
        assert_eq!(doc, json!({"config": {"profiles": [{"name": "work"}]}}));
        assert!(doc["config"]["profiles"].is_array());
    }

    /// The settings window must hand back the user's own document, not the
    /// defaults-merged one: saving the merge pins every current default into
    /// the file, so later releases can never change them.
    #[test]
    fn drafts_start_from_the_user_document_and_only_display_defaults() {
        let mut w = SettingsWindow {
            doc: json!({"config": {"fontSize": 18}}),
            saved_doc: json!({"config": {"fontSize": 18}}),
            defaults: json!({"config": {"fontSize": 12, "cursorShape": "BLOCK"}}),
            ..Default::default()
        };

        // The user's value wins; an undeclared key falls back to the default…
        assert_eq!(w.effective(&["config", "fontSize"]), Some(&json!(18)));
        assert_eq!(
            w.effective(&["config", "cursorShape"]),
            Some(&json!("BLOCK"))
        );
        // …but only for display — it is not in the document that gets saved.
        assert!(get_path(&w.doc, &["config", "cursorShape"]).is_none());
        assert!(!w.visual_dirty());

        // Resetting removes the key rather than writing the default in.
        remove_path(&mut w.doc, &["config", "fontSize"]);
        assert_eq!(w.effective(&["config", "fontSize"]), Some(&json!(12)));
        assert!(w.visual_dirty());
        assert_eq!(w.doc, json!({"config": {}}));
    }

    /// Switching tabs carries the draft across; an unparseable JSON draft
    /// blocks the switch instead of being thrown away.
    #[test]
    fn tab_switch_syncs_drafts_and_refuses_invalid_json() {
        let mut w = SettingsWindow {
            doc: json!({"config": {"fontSize": 18}}),
            saved_doc: json!({"config": {"fontSize": 18}}),
            ..Default::default()
        };

        w.switch_to(Section::Json);
        assert_eq!(w.section as u8, Section::Json as u8);
        assert!(w.json_text.contains("\"fontSize\": 18"));

        // A valid edit comes back through to the visual editor.
        w.json_text = json!({"config": {"fontSize": 20}}).to_string();
        w.switch_to(Section::Terminal);
        assert_eq!(get_path(&w.doc, &["config", "fontSize"]), Some(&json!(20)));

        // A broken one keeps the user on the JSON tab with their text intact.
        w.switch_to(Section::Json);
        w.json_text = "{\"config\": ".into();
        w.switch_to(Section::Terminal);
        assert_eq!(w.section as u8, Section::Json as u8);
        assert!(w.json_error.is_some());
        assert_eq!(w.json_text, "{\"config\": ");
    }

    /// A command bound to several chords survives a round trip through the
    /// keymap editor's single-line field.
    #[test]
    fn keymap_lists_stay_lists() {
        let value = json!(["command+t", "ctrl+t"]);
        let text = value
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(text, "command+t, ctrl+t");

        let parts: Vec<&str> = text.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        assert_eq!(json!(parts), value);
    }
}
