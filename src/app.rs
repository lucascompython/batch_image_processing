use iced::widget::{
    Column, Space, button, checkbox, column, container, image as iced_image, pick_list,
    progress_bar, row, rule, scrollable, slider, text, text_input,
};
use iced::{Alignment, Color, Element, Font, Length, Task, Theme};
use std::path::PathBuf;

use crate::processing::batch::{BatchConfig, OutputFormat, ProcessResult};
use crate::processing::image_ops::{self, Rotation, TextColor, TextOverlayConfig, TextPosition};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PreviewCache {
    index: usize,
    rotation: Rotation,
    thumbnail: Arc<image::DynamicImage>,
    rotated_width: u32,
    rotated_height: u32,
}

// ── App State ────────────────────────────────────────────────────────────────

pub struct App {
    // Image list
    image_paths: Vec<PathBuf>,
    selected_index: Option<usize>,
    preview_handle: Option<iced_image::Handle>,

    // Processing settings
    quality: u8,
    rotation: Rotation,
    output_format: OutputFormat,
    output_dir: Option<PathBuf>,

    // Text overlay
    text_enabled: bool,
    text_template: String,
    text_position: TextPosition,
    text_font_size: f32,
    text_color_r: u8,
    text_color_g: u8,
    text_color_b: u8,

    // Batch processing state
    is_processing: bool,
    progress: f32,
    status_message: String,
    batch_results: Vec<ProcessResult>,
    show_color_picker: bool,
    saved_text_color: Option<(u8, u8, u8)>,
    preview_version: usize,
    preview_cache: Option<PreviewCache>,
}

// ── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    // File operations
    OpenFolder,
    FolderOpened(Option<Vec<PathBuf>>),
    SelectImage(usize),
    ImagePreviewLoaded(usize, Option<(PreviewCache, Vec<u8>, u32, u32)>),

    // Settings
    SetQuality(u8),
    SetRotation(Rotation),
    SetOutputFormat(OutputFormat),
    ChooseOutputDir,
    OutputDirChosen(Option<PathBuf>),

    // Text overlay
    ToggleText(bool),
    SetTextTemplate(String),
    SetTextPosition(TextPosition),
    SetTextFontSize(f32),
    ChooseTextColor,
    CancelColorPicker,
    SubmitColorPicker(Color),
    ColorChanged(Color),

    // Batch processing
    StartBatch,
    BatchComplete(Vec<ProcessResult>),
    BatchProgress(f32),
}

// ── Rotation / Position display helpers ──────────────────────────────────────

const ROTATION_OPTIONS: &[Rotation] = &[
    Rotation::None,
    Rotation::Cw90,
    Rotation::Cw180,
    Rotation::Cw270,
];

impl std::fmt::Display for Rotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Cw90 => write!(f, "90° CW"),
            Self::Cw180 => write!(f, "180°"),
            Self::Cw270 => write!(f, "270° CW"),
        }
    }
}

const POSITION_OPTIONS: &[TextPosition] = &[
    TextPosition::TopLeft,
    TextPosition::TopRight,
    TextPosition::BottomLeft,
    TextPosition::BottomRight,
    TextPosition::Center,
];

impl std::fmt::Display for TextPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "Top Left"),
            Self::TopRight => write!(f, "Top Right"),
            Self::BottomLeft => write!(f, "Bottom Left"),
            Self::BottomRight => write!(f, "Bottom Right"),
            Self::Center => write!(f, "Center"),
        }
    }
}

const OUTPUT_FORMAT_OPTIONS: &[OutputFormat] = &[OutputFormat::Jpeg, OutputFormat::Pdf];

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Pdf => write!(f, "PDF"),
        }
    }
}

// ── App implementation ───────────────────────────────────────────────────────

impl App {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                image_paths: Vec::new(),
                selected_index: None,
                preview_handle: None,
                quality: 70,
                rotation: Rotation::None,
                output_format: OutputFormat::Jpeg,
                output_dir: None,
                text_enabled: false,
                text_template: "{filename}".to_string(),
                text_position: TextPosition::BottomRight,
                text_font_size: 24.0,
                text_color_r: 255,
                text_color_g: 255,
                text_color_b: 255,
                is_processing: false,
                progress: 0.0,
                status_message: "Ready — Open a folder to begin".to_string(),
                batch_results: Vec::new(),
                show_color_picker: false,
                saved_text_color: None,
                preview_version: 0,
                preview_cache: None,
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFolder => {
                return Task::perform(
                    async {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select image folder")
                            .pick_folder()
                            .await;

                        if let Some(folder) = handle {
                            let dir = folder.path().to_path_buf();
                            let mut images: Vec<PathBuf> = std::fs::read_dir(&dir)
                                .ok()?
                                .filter_map(|e| e.ok())
                                .map(|e| e.path())
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
                            Some(images)
                        } else {
                            None
                        }
                    },
                    Message::FolderOpened,
                );
            }

            Message::FolderOpened(Some(paths)) => {
                self.status_message = format!("Loaded {} images", paths.len());
                self.image_paths = paths;
                self.selected_index = None;
                self.preview_handle = None;
                self.preview_cache = None;
                self.batch_results.clear();
            }
            Message::FolderOpened(None) => {}

            Message::SelectImage(idx) => {
                self.selected_index = Some(idx);
                return self.queue_preview_update();
            }

            Message::ImagePreviewLoaded(version, res) => {
                if let Some((cache, data, w, h)) = res {
                    if self.selected_index == Some(cache.index) && self.rotation == cache.rotation {
                        self.preview_cache = Some(cache);
                    }
                    if version == self.preview_version {
                        self.preview_handle = Some(iced_image::Handle::from_rgba(w, h, data));
                    }
                } else if version == self.preview_version {
                    self.preview_handle = None;
                }
            }

            Message::SetQuality(q) => {
                self.quality = q;
            }
            Message::SetRotation(r) => {
                self.rotation = r;
                return self.queue_preview_update();
            }
            Message::SetOutputFormat(f) => {
                self.output_format = f;
            }
            Message::ChooseOutputDir => {
                return Task::perform(
                    async {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select output folder")
                            .pick_folder()
                            .await;
                        handle.map(|h| h.path().to_path_buf())
                    },
                    Message::OutputDirChosen,
                );
            }
            Message::OutputDirChosen(dir) => {
                if let Some(ref d) = dir {
                    self.status_message = format!("Output: {}", d.display());
                }
                self.output_dir = dir;
            }

            // Text overlay
            Message::ToggleText(on) => {
                self.text_enabled = on;
                return self.queue_preview_update();
            }
            Message::SetTextTemplate(t) => {
                self.text_template = t;
                return self.queue_preview_update();
            }
            Message::SetTextPosition(p) => {
                self.text_position = p;
                return self.queue_preview_update();
            }
            Message::SetTextFontSize(s) => {
                self.text_font_size = s;
                return self.queue_preview_update();
            }
            Message::ChooseTextColor => {
                self.show_color_picker = true;
                self.saved_text_color =
                    Some((self.text_color_r, self.text_color_g, self.text_color_b));
            }
            Message::CancelColorPicker => {
                self.show_color_picker = false;
                if let Some((r, g, b)) = self.saved_text_color.take() {
                    self.text_color_r = r;
                    self.text_color_g = g;
                    self.text_color_b = b;
                    return self.queue_preview_update();
                }
            }
            Message::SubmitColorPicker(color) => {
                let [r, g, b, _a] = color.into_rgba8();
                self.text_color_r = r;
                self.text_color_g = g;
                self.text_color_b = b;
                self.show_color_picker = false;
                return self.queue_preview_update();
            }
            Message::ColorChanged(color) => {
                let [r, g, b, _a] = color.into_rgba8();
                self.text_color_r = r;
                self.text_color_g = g;
                self.text_color_b = b;
                return self.queue_preview_update();
            }

            // Batch processing
            Message::StartBatch => {
                if self.image_paths.is_empty() {
                    self.status_message = "No images loaded".to_string();
                    return Task::none();
                }
                let Some(ref output_dir) = self.output_dir else {
                    self.status_message = "Select an output folder first".to_string();
                    return Task::none();
                };

                self.is_processing = true;
                self.progress = 0.0;
                self.batch_results.clear();
                self.status_message = "Processing...".to_string();

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

                return Task::perform(
                    async move {
                        // Run the CPU-heavy batch in a blocking task so we
                        // don't starve the tokio runtime.
                        tokio::task::spawn_blocking(move || {
                            crate::processing::batch::process_batch(
                                &paths,
                                &config,
                                |_done, _total| {
                                    // Progress is reported via BatchComplete for now
                                },
                            )
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::BatchComplete,
                );
            }

            Message::BatchProgress(p) => {
                self.progress = p;
            }

            Message::BatchComplete(results) => {
                self.is_processing = false;
                self.progress = 1.0;
                let success_count = results.iter().filter(|r| r.success).count();
                let fail_count = results.len() - success_count;
                self.status_message = format!(
                    "Done — {success_count} succeeded, {fail_count} failed out of {} total",
                    results.len()
                );
                self.batch_results = results;
            }
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let sidebar = self.view_sidebar();
        let preview = self.view_preview();
        let settings = self.view_settings();

        let main_row = row![sidebar, preview, settings]
            .spacing(0)
            .height(Length::Fill);

        let status_bar = container(
            row![
                text(&self.status_message).size(13),
                Element::from(Space::new().width(Length::Fill)),
                if self.is_processing {
                    container(progress_bar(0.0..=1.0, self.progress))
                        .width(200.0)
                        .into()
                } else {
                    Element::from(text(""))
                }
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding(8)
        .style(container::dark);

        column![main_row, status_bar].into()
    }

    // ── Sub-views ────────────────────────────────────────────────────────

    fn view_sidebar(&self) -> Element<'_, Message> {
        let header = row![
            text("Images").size(16).font(Font::DEFAULT),
            Element::from(Space::new().width(Length::Fill)),
            button("📂 Open")
                .on_press(Message::OpenFolder)
                .padding([4, 12]),
        ]
        .align_y(Alignment::Center)
        .padding(8);

        let file_list: Element<Message> = if self.image_paths.is_empty() {
            container(
                text("No images loaded.\nClick Open to select a folder.")
                    .size(13)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
            )
            .padding(16)
            .center_x(Length::Fill)
            .into()
        } else {
            let items: Vec<Element<Message>> = self
                .image_paths
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("???");
                    let is_selected = self.selected_index == Some(i);
                    let label = text(name).size(13);
                    let btn = button(label)
                        .on_press(Message::SelectImage(i))
                        .width(Length::Fill)
                        .padding([4, 8]);

                    if is_selected {
                        container(btn)
                            .style(container::dark)
                            .width(Length::Fill)
                            .into()
                    } else {
                        container(btn).width(Length::Fill).into()
                    }
                })
                .collect();

            scrollable(Column::with_children(items).width(Length::Fill)).into()
        };

        container(
            column![header, Element::from(rule::horizontal(1)), file_list]
                .spacing(0)
                .height(Length::Fill),
        )
        .width(250)
        .height(Length::Fill)
        .style(container::bordered_box)
        .into()
    }

    fn view_preview(&self) -> Element<'_, Message> {
        let content: Element<Message> = if let Some(ref handle) = self.preview_handle {
            iced_image(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(
                text("Select an image to preview")
                    .size(16)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            )
            .center(Length::Fill)
            .into()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(4)
            .into()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        let section_title = |label: &str| -> Element<Message> {
            text(label.to_string())
                .size(14)
                .font(Font::DEFAULT)
                .color(Color::from_rgb(0.7, 0.8, 1.0))
                .into()
        };

        // ── Output Format ────────────────────────────────────────────
        let format_section = column![
            section_title("Output Format"),
            pick_list(
                OUTPUT_FORMAT_OPTIONS,
                Some(self.output_format),
                Message::SetOutputFormat
            )
            .width(Length::Fill),
        ]
        .spacing(4);

        // ── JPEG Quality ─────────────────────────────────────────────
        let quality_section = column![
            section_title(&format!("JPEG Quality: {}%", self.quality)),
            slider(1..=100, self.quality, Message::SetQuality).width(Length::Fill),
        ]
        .spacing(4);

        // ── Rotation ─────────────────────────────────────────────────
        let rotation_section = column![
            section_title("Rotation"),
            pick_list(ROTATION_OPTIONS, Some(self.rotation), Message::SetRotation)
                .width(Length::Fill),
        ]
        .spacing(4);

        // ── Text Overlay ─────────────────────────────────────────────
        let mut text_section = column![
            checkbox(self.text_enabled)
                .label("Add Text Overlay")
                .on_toggle(Message::ToggleText),
        ]
        .spacing(6);

        if self.text_enabled {
            text_section = text_section
                .push(
                    column![
                        section_title("Text ({filename} = file name)"),
                        text_input("e.g. {filename}", &self.text_template)
                            .on_input(Message::SetTextTemplate)
                            .size(13),
                    ]
                    .spacing(2),
                )
                .push(
                    column![
                        section_title("Position"),
                        pick_list(
                            POSITION_OPTIONS,
                            Some(self.text_position),
                            Message::SetTextPosition,
                        )
                        .width(Length::Fill),
                    ]
                    .spacing(2),
                )
                .push(
                    column![
                        section_title(&format!("Font Size: {:.0}px", self.text_font_size)),
                        slider(8.0..=200.0, self.text_font_size, Message::SetTextFontSize)
                            .width(Length::Fill),
                    ]
                    .spacing(2),
                )
                .push(
                    column![
                        section_title("Text Color"),
                        button(text(format!(
                            "RGB({}, {}, {})",
                            self.text_color_r, self.text_color_g, self.text_color_b
                        )))
                        .on_press(Message::ChooseTextColor)
                        .width(Length::Fill),
                    ]
                    .spacing(2),
                );
        }

        // ── Output Dir ───────────────────────────────────────────────
        let output_label = if let Some(ref dir) = self.output_dir {
            dir.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Selected")
                .to_string()
        } else {
            "Not set".to_string()
        };

        let output_section = column![
            section_title("Output Folder"),
            row![
                text(output_label).size(13),
                Element::from(Space::new().width(Length::Fill)),
                button("Browse")
                    .on_press(Message::ChooseOutputDir)
                    .padding([4, 8]),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(4);

        // ── Actions ──────────────────────────────────────────────────
        let process_button = if self.is_processing {
            button("Processing...").width(Length::Fill).padding(10)
        } else {
            button("Process All")
                .on_press(Message::StartBatch)
                .width(Length::Fill)
                .padding(10)
        };

        // ── Results summary ──────────────────────────────────────────
        let results_section: Element<Message> = if self.batch_results.is_empty() {
            text("").into()
        } else {
            let failed: Vec<_> = self.batch_results.iter().filter(|r| !r.success).collect();
            if failed.is_empty() {
                text("All images processed successfully!")
                    .size(13)
                    .color(Color::from_rgb(0.3, 0.9, 0.3))
                    .into()
            } else {
                let error_list: Vec<Element<Message>> = failed
                    .iter()
                    .take(5)
                    .map(|r| {
                        text(format!(
                            "❌ {}: {}",
                            r.source.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                            r.error.as_deref().unwrap_or("unknown")
                        ))
                        .size(11)
                        .color(Color::from_rgb(1.0, 0.4, 0.4))
                        .into()
                    })
                    .collect();
                Column::with_children(error_list).spacing(2).into()
            }
        };

        let settings_col = column![
            format_section,
            Element::from(rule::horizontal(1)),
            quality_section,
            Element::from(rule::horizontal(1)),
            rotation_section,
            Element::from(rule::horizontal(1)),
            text_section,
            Element::from(rule::horizontal(1)),
            output_section,
            Element::from(rule::horizontal(1)),
            process_button,
            results_section,
        ]
        .spacing(10)
        .padding(12);

        let settings_container = container(scrollable(settings_col))
            .width(280)
            .height(Length::Fill)
            .style(container::bordered_box);

        let current_color =
            Color::from_rgb8(self.text_color_r, self.text_color_g, self.text_color_b);

        iced_aw::helpers::color_picker_with_change(
            self.show_color_picker,
            current_color,
            settings_container,
            Message::CancelColorPicker,
            Message::SubmitColorPicker,
            Message::ColorChanged,
        )
        .into()
    }

    fn build_text_config(&self) -> TextOverlayConfig {
        TextOverlayConfig {
            text_template: self.text_template.clone(),
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

    fn queue_preview_update(&mut self) -> Task<Message> {
        let Some(idx) = self.selected_index else {
            return Task::none();
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

        let cached = if let Some(ref c) = self.preview_cache {
            if c.index == idx && c.rotation == rotation {
                Some(c.clone())
            } else {
                None
            }
        } else {
            None
        };

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let cache = if let Some(c) = cached {
                        c
                    } else {
                        let mut img = image_ops::load_image(&path).ok()?;
                        if rotation != Rotation::None {
                            img = image_ops::rotate_image(&img, rotation);
                        }
                        let orig_w = img.width();
                        let orig_h = img.height();
                        // Generate thumbnail
                        let thumb = image_ops::generate_thumbnail(&img, 1200);
                        let thumb_dyn = image::DynamicImage::ImageRgba8(thumb);

                        PreviewCache {
                            index: idx,
                            rotation,
                            thumbnail: Arc::new(thumb_dyn),
                            rotated_width: orig_w,
                            rotated_height: orig_h,
                        }
                    };

                    let mut final_thumb = (*cache.thumbnail).clone();
                    if let Some(mut tc) = text_config {
                        let scale = final_thumb.width() as f32 / cache.rotated_width.max(1) as f32;
                        tc.font_size *= scale;
                        tc.margin = (tc.margin as f32 * scale) as u32;

                        let filename = path.file_stem().and_then(|n| n.to_str()).unwrap_or("image");
                        final_thumb = image_ops::overlay_text(&final_thumb, &tc, filename);
                    }

                    let rgba = final_thumb.into_rgba8();
                    let w = rgba.width();
                    let h = rgba.height();
                    Some((cache, rgba.into_raw(), w, h))
                })
                .await
                .unwrap_or(None)
            },
            move |res| Message::ImagePreviewLoaded(version, res),
        )
    }
}
