//! COSMIC panel applet showing workspaces as dots or squares.
mod app;
mod wayland;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<app::AppModel>(())
}
