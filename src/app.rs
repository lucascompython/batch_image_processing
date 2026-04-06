use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::IndexPath;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::progress::Progress;
use gpui_component::scroll::ScrollableElement;
use gpui_component::select::{SelectEvent, SelectItem, SelectState};
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use std::path::PathBuf;
use std::sync::Arc;

use crate::numbering_mode::NumberingMode;
use crate::processing::batch::{BatchConfig, OutputFormat, ProcessResult};
use crate::processing::image_cache::ImageCache;
use crate::processing::image_ops::{Rotation, TextColor, TextOverlayConfig, TextPosition};

// ---------------------------------------------------------------------------
// SelectItem wrapper
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct SelectOption<V: Clone> {
    label: SharedString,
    value: V,
}

impl<V: Clone + 'static> SelectItem for SelectOption<V> {
    type Value = V;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &V {
        &self.value
    }
}

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

type FormatSelectState = SelectState<Vec<SelectOption<OutputFormat>>>;
type RotationSelectState = SelectState<Vec<SelectOption<Rotation>>>;
type PositionSelectState = SelectState<Vec<SelectOption<TextPosition>>>;

// ---------------------------------------------------------------------------
// App Mode
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppMode {
    BatchProcessing,
    Numbering,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    // Current mode
    mode: AppMode,
    numbering_mode: Entity<NumberingMode>,

    // Shared image cache
    image_cache: Arc<ImageCache>,

    // Image list (batch mode)
    image_paths: Vec<PathBuf>,
    selected_index: Option<usize>,
    preview_path: Option<PathBuf>,
    preview_version: usize,

    // Settings (batch mode)
    quality: u8,
    rotation: Rotation,
    output_format: OutputFormat,
    output_dir: Option<PathBuf>,

    // Text overlay
    text_enabled: bool,
    text_position: TextPosition,
    text_font_size: f32,
    text_color_r: u8,
    text_color_g: u8,
    text_color_b: u8,

    // Processing state
    is_processing: bool,
    progress: f32,
    status_message: SharedString,
    batch_results: Vec<ProcessResult>,

    // Entity handles
    quality_slider: Entity<SliderState>,
    font_size_slider: Entity<SliderState>,
    format_select: Entity<FormatSelectState>,
    rotation_select: Entity<RotationSelectState>,
    position_select: Entity<PositionSelectState>,
    text_input: Entity<InputState>,
    color_picker: Entity<ColorPickerState>,
    text_template_value: String,

    _subscriptions: Vec<Subscription>,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Shared image cache
        let image_cache = Arc::new(ImageCache::new(15));

        // Numbering mode entity
        let numbering_mode = {
            let cache = image_cache.clone();
            cx.new(|cx| NumberingMode::new(window, cx, cache))
        };

        // Quality slider: 1..100, default 70
        let quality_slider = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(100.0)
                .step(1.0)
                .default_value(70.0)
        });

        // Font size slider: 8..200, default 24
        let font_size_slider = cx.new(|_| {
            SliderState::new()
                .min(8.0)
                .max(200.0)
                .step(1.0)
                .default_value(24.0)
        });

        // Format select
        let format_items = vec![
            SelectOption {
                label: "JPEG".into(),
                value: OutputFormat::Jpeg,
            },
            SelectOption {
                label: "PDF".into(),
                value: OutputFormat::Pdf,
            },
        ];
        let format_select = cx.new(|cx| {
            SelectState::new(format_items, Some(IndexPath::default().row(0)), window, cx)
        });

        // Rotation select
        let rotation_items = vec![
            SelectOption {
                label: "None".into(),
                value: Rotation::None,
            },
            SelectOption {
                label: "90° CW".into(),
                value: Rotation::Cw90,
            },
            SelectOption {
                label: "180°".into(),
                value: Rotation::Cw180,
            },
            SelectOption {
                label: "270° CW".into(),
                value: Rotation::Cw270,
            },
        ];
        let rotation_select = cx.new(|cx| {
            SelectState::new(
                rotation_items,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });

        // Position select
        let position_items = vec![
            SelectOption {
                label: "Top Left".into(),
                value: TextPosition::TopLeft,
            },
            SelectOption {
                label: "Top Right".into(),
                value: TextPosition::TopRight,
            },
            SelectOption {
                label: "Bottom Left".into(),
                value: TextPosition::BottomLeft,
            },
            SelectOption {
                label: "Bottom Right".into(),
                value: TextPosition::BottomRight,
            },
            SelectOption {
                label: "Center".into(),
                value: TextPosition::Center,
            },
        ];
        let position_select = cx.new(|cx| {
            SelectState::new(
                position_items,
                Some(IndexPath::default().row(3)),
                window,
                cx,
            )
        });

        // Text input
        let text_input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. {filename}"));
        text_input.update(cx, |state, cx| {
            state.set_value("{filename}", window, cx);
        });

        // Color picker — default white
        let color_picker =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(hsla(0.0, 0.0, 1.0, 1.0)));

        // ---- Subscriptions ----
        let mut subs = Vec::new();

        // Quality slider
        subs.push(cx.subscribe_in(
            &quality_slider,
            window,
            |this, _, ev: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(val) = ev {
                    this.quality = val.start() as u8;
                    cx.notify();
                }
            },
        ));

        // Font size slider
        subs.push(cx.subscribe_in(
            &font_size_slider,
            window,
            |this, _, ev: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(val) = ev {
                    this.text_font_size = val.start();
                    this.schedule_preview_update(cx);
                    cx.notify();
                }
            },
        ));

        // Format select
        subs.push(cx.subscribe_in(
            &format_select,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<OutputFormat>>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = ev {
                    this.output_format = *value;
                    cx.notify();
                }
            },
        ));

        // Rotation select
        subs.push(cx.subscribe_in(
            &rotation_select,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<Rotation>>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = ev {
                    this.rotation = *value;
                    this.schedule_preview_update(cx);
                    cx.notify();
                }
            },
        ));

        // Position select
        subs.push(cx.subscribe_in(
            &position_select,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<TextPosition>>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = ev {
                    this.text_position = *value;
                    this.schedule_preview_update(cx);
                    cx.notify();
                }
            },
        ));

        // Text input
        subs.push(cx.subscribe_in(
            &text_input,
            window,
            |this, state, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    let val = state.read(cx).value();
                    this.text_template_value = val.to_string();
                    this.schedule_preview_update(cx);
                    cx.notify();
                }
                _ => {}
            },
        ));

        // Color picker
        subs.push(cx.subscribe_in(
            &color_picker,
            window,
            |this, _, ev: &ColorPickerEvent, _window, cx| {
                if let ColorPickerEvent::Change(Some(hsla)) = ev {
                    let (r, g, b) = hsla_to_rgb(hsla.h, hsla.s, hsla.l);
                    this.text_color_r = r;
                    this.text_color_g = g;
                    this.text_color_b = b;
                    this.schedule_preview_update(cx);
                    cx.notify();
                }
            },
        ));

        Self {
            mode: AppMode::BatchProcessing,
            numbering_mode,
            image_cache,

            image_paths: Vec::new(),
            selected_index: None,
            preview_path: None,
            preview_version: 0,

            quality: 70,
            rotation: Rotation::None,
            output_format: OutputFormat::Jpeg,
            output_dir: None,

            text_enabled: false,
            text_position: TextPosition::BottomRight,
            text_font_size: 24.0,
            text_color_r: 255,
            text_color_g: 255,
            text_color_b: 255,

            is_processing: false,
            progress: 0.0,
            status_message: "Ready — Open a folder to begin".into(),
            batch_results: Vec::new(),

            quality_slider,
            font_size_slider,
            format_select,
            rotation_select,
            position_select,
            text_input,
            color_picker,
            text_template_value: "{filename}".to_string(),

            _subscriptions: subs,
        }
    }

    // -----------------------------------------------------------------------
    // Mode switching
    // -----------------------------------------------------------------------

    fn set_mode(&mut self, mode: AppMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, mut cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("Select image folder")
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

                _ = this.update(cx, |this, cx| {
                    this.status_message = format!("Loaded {} images", images.len()).into();
                    this.image_paths = images;
                    this.selected_index = None;
                    this.preview_path = None;
                    this.batch_results.clear();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn select_image(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected_index = Some(idx);
        self.schedule_preview_update(cx);
        cx.notify();
    }

    fn choose_output_dir(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, mut cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("Select output folder")
                .pick_folder()
                .await;
            if let Some(folder) = handle {
                let dir = folder.path().to_path_buf();
                _ = this.update(cx, |this, cx| {
                    this.status_message = format!("Output: {}", dir.display()).into();
                    this.output_dir = Some(dir);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_batch(&mut self, cx: &mut Context<Self>) {
        if self.image_paths.is_empty() {
            self.status_message = "No images loaded".into();
            cx.notify();
            return;
        }
        let Some(ref output_dir) = self.output_dir else {
            self.status_message = "Select an output folder first".into();
            cx.notify();
            return;
        };

        self.is_processing = true;
        self.progress = 0.0;
        self.batch_results.clear();
        self.status_message = "Processing...".into();
        cx.notify();

        let paths = self.image_paths.clone();
        let config = BatchConfig {
            quality: self.quality,
            rotation: self.rotation,
            text_overlay: if self.text_enabled {
                Some(self.build_text_config())
            } else {
                None
            },
            output_format: self.output_format,
            output_dir: output_dir.clone(),
        };

        let executor = cx.background_executor().clone();

        let entity = cx.entity().downgrade();
        cx.spawn(async move |this, mut cx| {
            let results = executor
                .spawn(async move {
                    crate::processing::batch::process_batch(&paths, &config, |_, _| {})
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.is_processing = false;
                this.progress = 1.0;
                let success = results.iter().filter(|r| r.success).count();
                let fail = results.len() - success;
                this.status_message = format!(
                    "Done — {success} succeeded, {fail} failed out of {} total",
                    results.len()
                )
                .into();
                this.batch_results = results;
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_preview_update(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.selected_index else {
            return;
        };
        self.preview_version = self.preview_version.wrapping_add(1);
        let version = self.preview_version;

        let path = self.image_paths[idx].clone();
        let rotation = self.rotation;
        let text_config = if self.text_enabled {
            Some(self.build_text_config())
        } else {
            None
        };

        let cache = self.image_cache.clone();

        cx.spawn(async move |this, mut cx| {
            // Use cache to get decoded image, then save to temp file for GPUI
            let result: Option<PathBuf> = (|| {
                let cached = cache.get_or_decode(&path, rotation, text_config.as_ref())?;
                let temp_path = std::env::temp_dir().join(format!("bip_preview_{version}.png"));
                cached
                    .rgba
                    .save_with_format(&temp_path, image::ImageFormat::Png)
                    .ok()?;
                Some(temp_path)
            })();

            _ = this.update(cx, |this, cx| {
                if version == this.preview_version {
                    this.preview_path = result;
                }
                cx.notify();
            });
        })
        .detach();

        // Preload adjacent images in background
        self.preload_adjacent(cx);
    }

    fn preload_adjacent(&self, cx: &mut Context<Self>) {
        let Some(idx) = self.selected_index else {
            return;
        };

        let paths = &self.image_paths;
        let mut adjacent = Vec::new();
        if idx > 0 {
            adjacent.push(paths[idx - 1].clone());
        }
        if idx + 1 < paths.len() {
            adjacent.push(paths[idx + 1].clone());
        }

        if adjacent.is_empty() {
            return;
        }

        let cache = self.image_cache.clone();
        let rotation = self.rotation;
        let text_config = if self.text_enabled {
            Some(self.build_text_config())
        } else {
            None
        };

        cx.background_executor()
            .spawn(async move {
                cache.preload(&adjacent, rotation, text_config.as_ref());
            })
            .detach();
    }

    fn build_text_config(&self) -> TextOverlayConfig {
        TextOverlayConfig {
            text_template: self.text_template_value.clone(),
            position: self.text_position,
            font_size: self.text_font_size,
            color: TextColor {
                r: self.text_color_r,
                g: self.text_color_g,
                b: self.text_color_b,
                a: 255,
            },
            margin: 10,
        }
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();

        let header = h_flex()
            .px_3()
            .py_2()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Images"),
            )
            .child(Button::new("open-btn").label("📂 Open").small().on_click({
                let entity = entity.clone();
                move |_, _window, cx| {
                    entity.update(cx, |this, cx| this.open_folder(cx));
                }
            }));

        let file_list = if self.image_paths.is_empty() {
            div()
                .p_4()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No images loaded.\nClick Open to select a folder."),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .overflow_y_scrollbar()
                .children(self.image_paths.iter().enumerate().map(|(i, path)| {
                    let name: SharedString = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("???")
                        .to_string()
                        .into();
                    let is_selected = self.selected_index == Some(i);
                    let entity = entity.clone();

                    div()
                        .id(("file-item", i))
                        .px_2()
                        .py_1()
                        .w_full()
                        .text_sm()
                        .cursor_pointer()
                        .when(is_selected, |el| {
                            el.bg(cx.theme().accent)
                                .text_color(cx.theme().accent_foreground)
                        })
                        .when(!is_selected, |el| el.hover(|el| el.bg(cx.theme().muted)))
                        .on_click(move |_, _window, cx| {
                            entity.update(cx, |this, cx| this.select_image(i, cx));
                        })
                        .child(name)
                }))
                .into_any_element()
        };

        v_flex()
            .w(px(250.))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(header)
            .child(div().h(px(1.)).bg(cx.theme().border))
            .child(file_list)
    }

    fn render_preview(&self, _cx: &mut Context<Self>) -> impl IntoElement {
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
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_base()
                        .text_color(gpui::hsla(0., 0., 0.4, 1.0))
                        .child("Select an image to preview"),
                )
                .into_any_element()
        };

        div().flex_1().h_full().p_1().child(content)
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();
        let section_title = |label: &str| -> AnyElement {
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().accent)
                .child(label.to_string())
                .into_any_element()
        };

        // Format section
        let format_section = v_flex()
            .gap_1()
            .child(section_title("Output Format"))
            .child(self.format_select.clone());

        // Quality section
        let quality_section = v_flex()
            .gap_1()
            .child(section_title(&format!("JPEG Quality: {}%", self.quality)))
            .child(Slider::new(&self.quality_slider).w_full());

        // Rotation section
        let rotation_section = v_flex()
            .gap_1()
            .child(section_title("Rotation"))
            .child(self.rotation_select.clone());

        // Text overlay section
        let mut text_section = v_flex().gap_2().child(
            Checkbox::new("text-enabled")
                .checked(self.text_enabled)
                .label("Add Text Overlay")
                .on_click({
                    let entity = entity.clone();
                    move |checked, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.text_enabled = *checked;
                            this.schedule_preview_update(cx);
                            cx.notify();
                        });
                    }
                }),
        );

        if self.text_enabled {
            text_section = text_section
                .child(
                    v_flex()
                        .gap_1()
                        .child(section_title("Text ({filename} = file name)"))
                        .child(Input::new(&self.text_input).small()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(section_title("Position"))
                        .child(self.position_select.clone()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(section_title(&format!(
                            "Font Size: {:.0}px",
                            self.text_font_size
                        )))
                        .child(Slider::new(&self.font_size_slider).w_full()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(section_title("Text Color"))
                        .child(ColorPicker::new(&self.color_picker)),
                );
        }

        // Output folder section
        let output_label: SharedString = if let Some(ref dir) = self.output_dir {
            dir.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Selected")
                .to_string()
                .into()
        } else {
            "Not set".into()
        };

        let output_section = v_flex()
            .gap_1()
            .child(section_title("Output Folder"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().child(output_label))
                    .child(div().flex_1())
                    .child(Button::new("browse-btn").label("Browse").small().on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.choose_output_dir(cx));
                        }
                    })),
            );

        // Process button
        let process_button = if self.is_processing {
            Button::new("process-btn")
                .label("Processing...")
                .w_full()
                .disabled(true)
        } else {
            Button::new("process-btn")
                .label("Process All")
                .primary()
                .w_full()
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.start_batch(cx));
                    }
                })
        };

        // Results
        let results_el = if self.batch_results.is_empty() {
            div().into_any_element()
        } else {
            let failed: Vec<_> = self.batch_results.iter().filter(|r| !r.success).collect();
            if failed.is_empty() {
                div()
                    .text_sm()
                    .text_color(gpui::hsla(0.33, 0.9, 0.4, 1.0))
                    .child("All images processed successfully!")
                    .into_any_element()
            } else {
                v_flex()
                    .gap_0p5()
                    .children(failed.iter().take(5).map(|r| {
                        let name = r.source.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let err = r.error.as_deref().unwrap_or("unknown");
                        div()
                            .text_xs()
                            .text_color(gpui::hsla(0.0, 0.8, 0.5, 1.0))
                            .child(format!("❌ {name}: {err}"))
                    }))
                    .into_any_element()
            }
        };

        let divider = || div().h(px(1.)).bg(cx.theme().border);

        let settings_col = v_flex()
            .gap_3()
            .p_3()
            .child(format_section)
            .child(divider())
            .child(quality_section)
            .child(divider())
            .child(rotation_section)
            .child(divider())
            .child(text_section)
            .child(divider())
            .child(output_section)
            .child(divider())
            .child(process_button)
            .child(results_el);

        v_flex()
            .w(px(280.))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .overflow_y_scrollbar()
            .child(settings_col)
    }
}

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;
        let entity = cx.entity().clone();
        let is_batch = mode == AppMode::BatchProcessing;
        let is_numbering = mode == AppMode::Numbering;

        // Inline tab rendering to avoid borrow issues
        let tab_style = |active: bool, theme: &gpui_component::theme::Theme| {
            let base = div().px_4().py_2().text_sm().cursor_pointer().border_b_2();
            if active {
                base.border_color(theme.accent)
                    .text_color(theme.foreground)
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                base.border_color(transparent_black())
                    .text_color(theme.muted_foreground)
                    .hover(|el| el.text_color(theme.foreground))
            }
        };

        let theme = cx.theme().clone();
        let tabs = h_flex()
            .bg(theme.background)
            .border_b_1()
            .border_color(theme.border)
            .child(
                tab_style(is_batch, &theme)
                    .id("tab-batch")
                    .child("📦 Batch Processing")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity
                                .update(cx, |this, cx| this.set_mode(AppMode::BatchProcessing, cx));
                        }
                    }),
            )
            .child(
                tab_style(is_numbering, &theme)
                    .id("tab-numbering")
                    .child("🏍️ Numbering")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.set_mode(AppMode::Numbering, cx));
                        }
                    }),
            );

        let content = match mode {
            AppMode::BatchProcessing => {
                let main_row = h_flex()
                    .flex_1()
                    .child(self.render_sidebar(cx))
                    .child(self.render_preview(cx))
                    .child(self.render_settings(cx));

                let status_bar = h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .bg(cx.theme().background)
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.status_message.clone()),
                    )
                    .child(div().flex_1())
                    .when(self.is_processing, |el| {
                        el.child(
                            div()
                                .w(px(200.))
                                .child(Progress::new("progress").value(self.progress * 100.0)),
                        )
                    });

                v_flex()
                    .flex_1()
                    .child(main_row)
                    .child(status_bar)
                    .into_any_element()
            }
            AppMode::Numbering => div()
                .flex_1()
                .child(self.numbering_mode.clone())
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(tabs)
            .child(content)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hsla_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h6 = h * 6.0;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r1, g1, b1) = if h6 < 1.0 {
        (c, x, 0.0)
    } else if h6 < 2.0 {
        (x, c, 0.0)
    } else if h6 < 3.0 {
        (0.0, c, x)
    } else if h6 < 4.0 {
        (0.0, x, c)
    } else if h6 < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}
