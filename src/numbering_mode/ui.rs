//! Numbering mode UI component.

use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{ActiveTheme, ElementExt, Sizable, h_flex, v_flex};
use std::path::PathBuf;
use std::sync::Arc;

use super::state::{ConfidenceLevel, NumberingState};
use crate::processing::image_cache::ImageCache;

/// NumberingMode component that handles the image numbering workflow.
pub struct NumberingMode {
    state: NumberingState,
    input_state: Entity<InputState>,
    image_cache: Arc<ImageCache>,
    preview_image: Option<Arc<Image>>,
    preview_dimensions: Option<(u32, u32)>,
    image_view_size: Option<(f32, f32)>,
    preview_version: usize,
    _subscriptions: Vec<Subscription>,
}

impl NumberingMode {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, image_cache: Arc<ImageCache>) -> Self {
        let input_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type motorcycle number..."));

        let mut subs = Vec::new();

        subs.push(cx.subscribe_in(
            &input_state,
            window,
            |this, _state, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.confirm_and_advance(window, cx);
                }
            },
        ));

        Self {
            state: NumberingState::new(),
            input_state,
            image_cache,
            preview_image: None,
            preview_dimensions: None,
            image_view_size: None,
            preview_version: 0,
            _subscriptions: subs,
        }
    }

    pub fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity().clone();
        cx.spawn_in(window, async move |_this, cx| {
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

                entity.update(cx, |this, cx| {
                    this.state.source_folder = Some(dir);
                    this.state.image_paths = images;
                    this.state.current_index = 0;
                    this.state.undo_stack.clear();
                    this.state.input_buffer.clear();
                    this.state.status_message =
                        format!("Loaded {} images", this.state.image_paths.len());

                    // load first image
                    this.load_current_image(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_sticker_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity().clone();
        cx.spawn_in(window, async move |_this, cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("Select event sticker template")
                .add_filter("Images", &["png", "jpg", "jpeg"])
                .pick_file()
                .await;

            if let Some(file) = handle {
                let path = file.path().to_path_buf();
                let load_result = crate::ocr::set_sticker_template(&path);

                entity.update(cx, |this, cx| {
                    match load_result {
                        Ok(()) => {
                            let label = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("template");
                            this.state.status_message = format!("Sticker template loaded: {label}");
                            this.load_current_image(cx);
                        }
                        Err(err) => {
                            this.state.status_message = format!("Template load failed: {err}");
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn clear_sticker_template(&mut self, cx: &mut Context<Self>) {
        crate::ocr::clear_sticker_template();
        self.state.status_message = "Sticker template cleared".into();
        self.load_current_image(cx);
        cx.notify();
    }

    fn load_current_image(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.state.current_image().cloned() {
            self.preview_version = self.preview_version.wrapping_add(1);
            let version = self.preview_version;
            let cache = self.image_cache.clone();
            let ocr_enabled = crate::ocr::is_ocr_available();
            let preview_path = path.clone();
            self.state.pan_x = 0.0;
            self.state.pan_y = 0.0;
            self.state.is_dragging = false;

            self.state.ocr_running = ocr_enabled;
            self.state.ocr_suggestion = None;

            // load and present preview as soon as possible.
            cx.spawn(async move |this, cx| {
                let still_current = this
                    .update(cx, |this, _| version == this.preview_version)
                    .unwrap_or(false);
                if !still_current {
                    return;
                }

                let (preview, dimensions) = {
                    let cached = cache.get_or_decode(
                        &preview_path,
                        crate::processing::image_ops::Rotation::None,
                        None,
                    );

                    cached
                        .as_ref()
                        .and_then(|c| {
                            let mut bytes = Vec::new();
                            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                                &mut bytes,
                                image::codecs::png::CompressionType::Fast,
                                image::codecs::png::FilterType::NoFilter,
                            );
                            image::ImageEncoder::write_image(
                                encoder,
                                c.rgba.as_raw(),
                                c.rgba.width(),
                                c.rgba.height(),
                                image::ColorType::Rgba8.into(),
                            )
                            .ok()?;
                            Some((
                                Arc::new(Image::from_bytes(gpui::ImageFormat::Png, bytes)),
                                (c.rgba.width(), c.rgba.height()),
                            ))
                        })
                        .map_or((None, None), |(img, dims)| (Some(img), Some(dims)))
                };

                _ = this.update(cx, |this, cx| {
                    if version == this.preview_version {
                        this.preview_image = preview;
                        this.preview_dimensions = dimensions;
                    }
                    cx.notify();
                });
            })
            .detach();

            // OCR runs independently so image transitions stay snappy
            if ocr_enabled {
                let ocr_path = path.clone();
                cx.spawn(async move |this, cx| {
                    let still_current = this
                        .update(cx, |this, _| version == this.preview_version)
                        .unwrap_or(false);
                    if !still_current {
                        return;
                    }

                    let ocr = crate::ocr::get_cached_ocr(&ocr_path).or_else(|| {
                        crate::processing::image_ops::load_image(&ocr_path)
                            .ok()
                            .and_then(|img| crate::ocr::recognize_number_for_path(&ocr_path, &img))
                    });

                    _ = this.update(cx, |this, cx| {
                        if version == this.preview_version {
                            this.state.ocr_running = false;
                            this.state.ocr_suggestion =
                                ocr.map(|value| super::state::OcrSuggestion {
                                    number: value.text,
                                    confidence: value.confidence,
                                });
                        }
                        cx.notify();
                    });
                })
                .detach();
            } else {
                self.state.ocr_running = false;
            }

            // Preload adjacent images and OCR
            self.preload_adjacent(cx);
        } else {
            self.preview_image = None;
            self.preview_dimensions = None;
            self.state.ocr_running = false;
            cx.notify();
        }
    }

    fn preload_adjacent(&self, cx: &mut Context<Self>) {
        let (image_preload, ocr_preload): (Vec<PathBuf>, Vec<PathBuf>) = {
            let idx = self.state.current_index;
            let paths = &self.state.image_paths;
            let mut img_adj = Vec::with_capacity(6);
            let mut ocr_adj = Vec::with_capacity(2);

            // Preload images 3 ahead and 2 behind
            for offset in 1..=3 {
                if idx + offset < paths.len() {
                    img_adj.push(paths[idx + offset].clone());
                }
            }
            for offset in 1..=2 {
                if idx >= offset {
                    img_adj.push(paths[idx - offset].clone());
                }
            }

            // Preload OCR for next 2 images only (OCR is expensive)
            for offset in 1..=2 {
                if idx + offset < paths.len() {
                    ocr_adj.push(paths[idx + offset].clone());
                }
            }

            (img_adj, ocr_adj)
        };

        if !image_preload.is_empty() {
            let cache = self.image_cache.clone();
            cx.background_executor()
                .spawn(async move {
                    cache.preload(
                        &image_preload,
                        crate::processing::image_ops::Rotation::None,
                        None,
                    );
                })
                .detach();
        }

        // Preload OCR results for next 2 images
        if crate::ocr::is_ocr_available() && !ocr_preload.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    crate::ocr::preload_ocr(&ocr_preload);
                })
                .detach();
        }
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

        if let Some((base_w, base_h)) = self.preview_dimensions {
            let (max_x, max_y) = self.pan_limits(base_w as f32, base_h as f32);
            self.state.pan_x = self.state.pan_x.clamp(-max_x, max_x);
            self.state.pan_y = self.state.pan_y.clamp(-max_y, max_y);
        }

        if self.state.zoom_level <= 1.01 {
            self.state.pan_x = 0.0;
            self.state.pan_y = 0.0;
            self.state.is_dragging = false;
        }
        cx.notify();
    }

    fn fitted_size(&self, base_w: f32, base_h: f32) -> (f32, f32) {
        let (view_w, view_h) = self.image_view_size.unwrap_or((base_w, base_h));
        if base_w <= 0.0 || base_h <= 0.0 || view_w <= 0.0 || view_h <= 0.0 {
            return (base_w.max(1.0), base_h.max(1.0));
        }

        let fit_scale = (view_w / base_w).min(view_h / base_h);
        (base_w * fit_scale, base_h * fit_scale)
    }

    fn pan_limits(&self, base_w: f32, base_h: f32) -> (f32, f32) {
        if self.state.zoom_level <= 1.01 {
            return (0.0, 0.0);
        }

        let (fit_w, fit_h) = self.fitted_size(base_w, base_h);
        let scaled_w = fit_w * self.state.zoom_level;
        let scaled_h = fit_h * self.state.zoom_level;
        (
            ((scaled_w - fit_w) * 0.5).max(0.0),
            ((scaled_h - fit_h) * 0.5).max(0.0),
        )
    }

    fn start_pan(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.state.zoom_level <= 1.01 {
            return;
        }
        self.state.is_dragging = true;
        self.state.drag_start_x = event.position.x.as_f32();
        self.state.drag_start_y = event.position.y.as_f32();
        cx.notify();
    }

    fn handle_pan_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.state.is_dragging {
            return;
        }

        let x = event.position.x.as_f32();
        let y = event.position.y.as_f32();
        let dx = x - self.state.drag_start_x;
        let dy = y - self.state.drag_start_y;
        self.state.drag_start_x = x;
        self.state.drag_start_y = y;

        self.state.pan_x += dx;
        self.state.pan_y += dy;

        if let Some((base_w, base_h)) = self.preview_dimensions {
            let (max_x, max_y) = self.pan_limits(base_w as f32, base_h as f32);
            self.state.pan_x = self.state.pan_x.clamp(-max_x, max_x);
            self.state.pan_y = self.state.pan_y.clamp(-max_y, max_y);
        }

        cx.notify();
    }

    fn end_pan(&mut self, cx: &mut Context<Self>) {
        if self.state.is_dragging {
            self.state.is_dragging = false;
            cx.notify();
        }
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
        let sticker_name = crate::ocr::sticker_template_name();
        let has_sticker = crate::ocr::has_sticker_template();

        let mut row = h_flex()
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
                Button::new("open-sticker-template")
                    .label("Sticker Template")
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| this.open_sticker_template(window, cx));
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
            }));

        if has_sticker {
            row = row.child(
                Button::new("clear-sticker-template")
                    .label("Clear Sticker")
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.clear_sticker_template(cx));
                        }
                    }),
            );
        }

        if let Some(name) = sticker_name {
            row = row.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Template: {name}")),
            );
        }

        row
    }

    fn render_image_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();

        let content = if let Some(ref preview) = self.preview_image {
            let source = gpui::ImageSource::Image(preview.clone());
            let (base_w, base_h) = self.preview_dimensions.unwrap_or((1200, 900));
            let (fit_w, fit_h) = self.fitted_size(base_w as f32, base_h as f32);
            let scaled_w = (fit_w * self.state.zoom_level).max(1.0);
            let scaled_h = (fit_h * self.state.zoom_level).max(1.0);
            let (max_pan_x, max_pan_y) = self.pan_limits(base_w as f32, base_h as f32);
            let pan_x = self.state.pan_x.clamp(-max_pan_x, max_pan_x);
            let pan_y = self.state.pan_y.clamp(-max_pan_y, max_pan_y);
            let (view_w, view_h) = self.image_view_size.unwrap_or((fit_w, fit_h));
            let left = ((view_w - scaled_w) * 0.5 + pan_x).round();
            let top = ((view_h - scaled_h) * 0.5 + pan_y).round();

            div().size_full().relative().overflow_hidden().child(
                img(source)
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(scaled_w))
                    .h(px(scaled_h))
                    .object_fit(ObjectFit::Fill),
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
            .min_w(px(0.))
            .min_h(px(0.))
            .w_full()
            .bg(hsla(0., 0., 0.1, 1.0))
            .overflow_hidden()
            .on_scroll_wheel({
                let entity = entity.clone();
                move |ev, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    let delta = ev.delta.pixel_delta(px(1.0)).y.as_f32();
                    entity.update(cx, |this, cx| this.handle_scroll(delta, cx));
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let entity = entity.clone();
                move |ev, _, cx| {
                    entity.update(cx, |this, cx| this.start_pan(ev, cx));
                }
            })
            .on_mouse_move({
                let entity = entity.clone();
                move |ev, _, cx| {
                    entity.update(cx, |this, cx| this.handle_pan_move(ev, cx));
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.end_pan(cx));
                }
            })
            .on_mouse_up_out(MouseButton::Left, {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.end_pan(cx));
                }
            })
            .on_prepaint({
                let entity = entity.clone();
                move |bounds, _, cx| {
                    entity.update(cx, |this, _| {
                        this.image_view_size =
                            Some((bounds.size.width.as_f32(), bounds.size.height.as_f32()));
                    });
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
                        .tab_index(-1)
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
                .child("Reading number...")
                .into_any_element()
        } else if !crate::ocr::is_ocr_available() {
            div()
                .text_sm()
                .text_color(hsla(0.0, 0.85, 0.6, 1.0))
                .child("OCR unavailable (missing model files)")
                .into_any_element()
        } else {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("OCR: no guess for this image")
                .into_any_element()
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
            .child(div().child(Input::new(&self.input_state).w(px(200.0)).tab_index(-1)))
            .child(
                Button::new("confirm")
                    .label("Enter ↵")
                    .small()
                    .primary()
                    .tab_index(-1)
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
                    .child(
                        "Scroll over image: zoom | Tab in number input: accept OCR | Ctrl+Z: undo",
                    ),
            )
    }
}

impl Render for NumberingMode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();

        v_flex()
            .size_full()
            .capture_key_down({
                let entity = entity.clone();
                move |ev, window, cx| {
                    let key = ev.keystroke.key.as_str();
                    let mods = ev.keystroke.modifiers;

                    if (mods.control || mods.platform) && key.eq_ignore_ascii_case("z") {
                        window.prevent_default();
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| this.undo(cx));
                    } else if key == "tab" {
                        window.prevent_default();
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| this.accept_ocr(window, cx));
                    }
                }
            })
            .child(self.render_toolbar(cx))
            .child(self.render_image_view(cx))
            .child(self.render_input_bar(cx))
            .child(self.render_status_bar(cx))
    }
}
