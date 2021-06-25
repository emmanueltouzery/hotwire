use super::win;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::path::PathBuf;

#[derive(Msg)]
pub enum Msg {}

pub struct Model {
    recent_file: PathBuf,
}

#[widget]
impl Widget for RecentFileItem {
    fn model(_relm: &relm::Relm<Self>, recent_file: PathBuf) -> Model {
        Model { recent_file }
    }

    fn update(&mut self, _event: Msg) {}

    view! {
        gtk::Box {
            margin_top: 7,
            margin_start: 7,
            margin_end: 7,
            margin_bottom: 7,
            property_width_request: 250,
            orientation: gtk::Orientation::Vertical,
            gtk::Label {
                halign: gtk::Align::Start,
                label: &self.model.recent_file
                            .file_name().and_then(|f| f.to_str()).unwrap_or(""),
                ellipsize: pango::EllipsizeMode::End,
            },
            #[style_class="recent_file_folder"]
            gtk::Label {
                halign: gtk::Align::Start,
                label: &self.model.recent_file
                            .parent().and_then(|f| f.to_str()).map(|f| f.trim_start_matches("file://")).unwrap_or(""),
                ellipsize: pango::EllipsizeMode::End,
                // can't get the folder name within a flatpak
                // https://github.com/flatpak/xdg-desktop-portal/issues/475
                visible: !win::is_flatpak(),
            },
        }
    }
}
