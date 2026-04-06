use gpui::*;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod app;
mod processing;

fn main() {
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        let opts = gpui::WindowOptions {
            ..Default::default()
        };

        cx.open_window(opts, |window, cx| {
            let view = cx.new(|cx| app::App::new(window, cx));

            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, Some(window), cx);
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .unwrap();
    });
}
