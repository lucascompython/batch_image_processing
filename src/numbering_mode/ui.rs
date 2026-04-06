//! Numbering mode UI component.

use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{ActiveTheme, Sizable, h_flex, v_flex};
use std::path::PathBuf;
use std::sync::Arc;

use super::state::{ConfidenceLevel, NumberingState};
use crate::processing::image_cache::ImageCache;

/// NumberingMode component that handles the image numbering workflow.
pub struct NumberingMode {
    state: NumberingState,
    input_state: Entity<InputState>,
    image_cache: Arc<ImageCache>,
    preview_path: Option<PathBuf>,
    preview_version: usize,
    _subscriptions: Vec<Subscription>,
}

impl NumberingMode {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, image_cache: Arc<ImageCache>) -> Self {
        let input_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type motorcycle number..."));

        let mut subs = Vec::new();

        // Subscribe to input changes
        subs.push(cx.subscribe_in(
            &input_state,
            window,
            |this, _state, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    this.confirm_and_advance(window, cx);
                }
                _ => {}
            },
        ));

        Self {
            state: NumberingState::new(),
            input_state,
            image_cache,
            preview_path: None,
            preview_version: 0,
            _subscriptions: subs,
        }
    }

    pub fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity().clone();
        cx.spawn_in(window, async move |_this, mut cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("Select image folder for numbering")
                .pick_folder()
                .await;

            if let Some(folder) = handle {
                let dir = folder.path().to_path_buf();
                let mut images: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flat_map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()))
                    .filter(|p| {
                        matches!(
                            p.extension()
                                .and_then(|e| e.to_str())
                                .map(|e| e.to_ascii_lowercase()),
                            Some(ref ext) if ext == "jpg" || ext == "jpeg" || ext == "png"
                        )
                    })
                    .collect();
                images.sort();

                _ = entity.update(cx, |this, cx| {
                    this.state.source_folder = Some(dir);
                    this.state.image_paths = images;
                    this.state.current_index = 0;
                    this.state.undo_stack.clear();
                    this.state.input_buffer.clear();
                    this.state.status_message =
                        format!("Loaded {} images", this.state.image_paths.len());

                    // Load first image
                    this.load_current_image(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn load_current_image(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.state.current_image().cloned() {
            self.preview_version = self.preview_version.wrapping_add(1);
            let version = self.preview_version;
            let cache = self.image_cache.clone();
            let ocr_path = path.clone();

            // Mark OCR as running
            self.state.ocr_running = true;
            self.state.ocr_suggestion = None;

            cx.spawn(async move |this, cx| {
                // Use cache to get or decode image, then save to temp file for GPUI
                let (preview_result, ocr_result) = {
                    let cached = cache.get_or_decode(
                        &path,
                        crate::processing::image_ops::Rotation::None,
                        None,
                    );

                    let preview = cached.as_ref().and_then(|c| {
                        let temp_path =
                            std::env::temp_dir().join(format!("bip_num_preview_{version}.png"));
                        c.rgba
                            .save_with_format(&temp_path, image::ImageFormat::Png)
                            .ok()?;
                        Some(temp_path)
                    });

                    // Run OCR on the full image (not thumbnail)
                    let ocr = if crate::ocr::is_ocr_available() {
                        if let Ok(img) = crate::processing::image_ops::load_image(&ocr_path) {
                            crate::ocr::recognize_number(&img)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    (preview, ocr)
                };

                _ = this.update(cx, |this, cx| {
                    if version == this.preview_version {
                        this.preview_path = preview_result;
                        this.state.ocr_running = false;

                        if let Some(ocr) = ocr_result {
                            this.state.ocr_suggestion = Some(super::state::OcrSuggestion {
                                number: ocr.text,
                                confidence: ocr.confidence,
                            });
                        }
                    }
                    cx.notify();
                });
            })
            .detach();

            // Preload adjacent images
            self.preload_adjacent(cx);
        } else {
            self.preview_path = None;
            cx.notify();
        }
    }

    fn preload_adjacent(&self, cx: &mut Context<Self>) {
        let adjacent: Vec<PathBuf> = {
            let idx = self.state.current_index;
            let paths = &self.state.image_paths;
            let mut adj = Vec::new();
            if idx > 0 {
                adj.push(paths[idx - 1].clone());
            }
            if idx + 1 < paths.len() {
                adj.push(paths[idx + 1].clone());
            }
            adj
        };

        if adjacent.is_empty() {
            return;
        }

        let cache = self.image_cache.clone();
        cx.background_executor()
            .spawn(async move {
                cache.preload(
                    &adjacent,
                    crate::processing::image_ops::Rotation::None,
                    None,
                );
            })
            .detach();
    }

    fn confirm_and_advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Get value from input state
        let value = self.input_state.read(cx).value().to_string();
        self.state.input_buffer = value;

        match self.state.confirm_number() {
            Ok(()) => {
                // Clear input and load next image
                self.input_state.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.load_current_image(cx);
            }
            Err(msg) => {
                self.state.status_message = format!("Error: {msg}");
            }
        }
        cx.notify();
    }

    fn skip_image(&mut self, cx: &mut Context<Self>) {
        self.state.next_image();
        self.load_current_image(cx);
        cx.notify();
    }

    fn prev_image(&mut self, cx: &mut Context<Self>) {
        self.state.prev_image();
        self.load_current_image(cx);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        match self.state.undo() {
            Ok(()) => {
                self.load_current_image(cx);
            }
            Err(msg) => {
                self.state.status_message = format!("Undo failed: {msg}");
            }
        }
        cx.notify();
    }

    fn handle_scroll(&mut self, delta: f32, cx: &mut Context<Self>) {
        if delta > 0.0 {
            self.state.zoom_in();
        } else {
            self.state.zoom_out();
        }
        cx.notify();
    }

    fn accept_ocr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref suggestion) = self.state.ocr_suggestion {
            let number = suggestion.number.clone();
            self.input_state.update(cx, |state, cx| {
                state.set_value(&number, window, cx);
            });
            self.state.input_buffer = number;
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();
        let (done, total) = self.state.progress();
        let remaining = self.state.remaining();

        h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .bg(cx.theme().background)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("open-folder")
                    .label("📂 Open Folder")
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| this.open_folder(window, cx));
                        }
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Progress: {done}/{total} ({remaining} remaining)")),
            )
            .child(div().flex_1())
            .child(Button::new("prev").label("← Prev").small().on_click({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.prev_image(cx));
                }
            }))
            .child(Button::new("skip").label("Skip →").small().on_click({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.skip_image(cx));
                }
            }))
            .child(Button::new("undo").label("↩ Undo").small().on_click({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.undo(cx));
                }
            }))
    }

    fn render_image_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();

        let content = if let Some(ref preview) = self.preview_path {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(gpui::ImageSource::Resource(gpui::Resource::Path(
                        preview.clone().into(),
                    )))
                    .max_w_full()
                    .max_h_full()
                    .object_fit(ObjectFit::Contain),
                )
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_base()
                        .text_color(hsla(0., 0., 0.5, 1.0))
                        .child("Open a folder to begin"),
                )
        };

        div()
            .id("image-view")
            .flex_1()
            .size_full()
            .bg(hsla(0., 0., 0.1, 1.0))
            .on_scroll_wheel({
                let entity = entity.clone();
                move |ev, _, cx| {
                    let delta = ev.delta.pixel_delta(px(1.0)).y.as_f32();
                    entity.update(cx, |this, cx| this.handle_scroll(delta, cx));
                }
            })
            .child(content)
    }

    fn render_input_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();

        let ocr_element = if let Some(ref suggestion) = self.state.ocr_suggestion {
            let (color, icon) = match suggestion.confidence_level() {
                ConfidenceLevel::High => (hsla(0.33, 0.8, 0.45, 1.0), "✓"),
                ConfidenceLevel::Medium => (hsla(0.12, 0.9, 0.5, 1.0), "⚠"),
                ConfidenceLevel::Low => (hsla(0.0, 0.8, 0.5, 1.0), "✗"),
            };
            let confidence_pct = (suggestion.confidence * 100.0) as u32;

            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("OCR:"),
                )
                .child(
                    Button::new("accept-ocr")
                        .label(format!("{} ({}%)", suggestion.number, confidence_pct))
                        .small()
                        .on_click({
                            let entity = entity.clone();
                            move |_, window, cx| {
                                entity.update(cx, |this, cx| this.accept_ocr(window, cx));
                            }
                        }),
                )
                .child(div().text_base().text_color(color).child(icon))
                .into_any_element()
        } else if self.state.ocr_running {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("🔄 Reading number...")
                .into_any_element()
        } else {
            div().into_any_element()
        };

        h_flex()
            .px_3()
            .py_2()
            .gap_4()
            .items_center()
            .bg(cx.theme().background)
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Number:"),
            )
            .child(Input::new(&self.input_state).w(px(200.0)))
            .child(
                Button::new("confirm")
                    .label("Enter ↵")
                    .small()
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| this.confirm_and_advance(window, cx));
                        }
                    }),
            )
            .child(div().flex_1())
            .child(ocr_element)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Zoom: {:.0}%", self.state.zoom_level * 100.0)),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_3()
            .py_1()
            .bg(cx.theme().muted)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.state.status_message.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Scroll: zoom | Drag: pan | Tab: accept OCR | Ctrl+Z: undo"),
            )
    }
}

impl Render for NumberingMode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(self.render_image_view(cx))
            .child(self.render_input_bar(cx))
            .child(self.render_status_bar(cx))
    }
}
