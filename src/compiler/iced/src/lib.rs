#[cfg(target_arch = "wasm32")]
use iced_ as public;

#[cfg(not(target_arch = "wasm32"))]
mod public {
    pub use iced_::*;

    pub fn run<State, Message>(
        update: impl application::UpdateFn<State, Message> + 'static,
        view: impl for<'a> application::ViewFn<'a, State, Message, Theme, Renderer> + 'static,
    ) -> Result
    where
        State: Default + 'static,
        Message: Send + message::MaybeDebug + message::MaybeClone + 'static,
    {
        fn save_png(screenshot: window::Screenshot, name: &str) {
            use std::fs;

            let file = fs::File::create(name).expect("should create screenshot");

            let mut encoder =
                png::Encoder::new(file, screenshot.size.width, screenshot.size.height);
            encoder.set_color(png::ColorType::Rgba);

            let mut writer = encoder.write_header().expect("should write PNG header");
            writer
                .write_image_data(&screenshot.rgba)
                .expect("should write PNG data");
            writer.finish().expect("should finish writing PNG");
        }

        let height = std::env::args()
            .skip(1)
            .next()
            .as_deref()
            .and_then(|height| height.parse().ok())
            .unwrap_or(200);

        let application =
            application(State::default, update, view).default_font(Font::with_name("Fira Sans"));

        for (name, theme) in [("light.png", Theme::Light), ("dark.png", Theme::Dark)] {
            let screenshot = iced_test::screenshot(
                &application,
                &theme,
                (730.0, height as f32),
                2.0,
                time::Duration::ZERO,
            );

            save_png(screenshot, name);
        }

        Ok(())
    }
}

pub use public::*;
