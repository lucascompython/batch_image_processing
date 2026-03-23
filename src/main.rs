mod app;
mod processing;

fn main() -> iced::Result {
    iced::application(app::App::new, app::App::update, app::App::view)
        .theme(app::App::theme)
        .window_size(iced::Size::new(1200.0, 750.0))
        .title("Batch Image Processing")
        .run()
}
