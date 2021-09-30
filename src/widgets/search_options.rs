use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg, Debug)]
pub enum Msg {}

pub struct Model {}

#[widget]
impl Widget for SearchOptions {
    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {}
    }

    fn update(&mut self, event: Msg) {
        match event {}
    }

    view! {
         gtk::Grid {
             orientation: gtk::Orientation::Vertical,
             margin_top: 10,
             margin_start: 10,
             margin_end: 10,
             margin_bottom: 10,
             row_spacing: 5,
             column_spacing: 10,
             #[style_class="label"]
             gtk::Label {
                 cell: {
                     left_attach: 0,
                     top_attach: 0,
                 },
                 text: "Grid cells",
                 halign: gtk::Align::End,
             },
             gtk::SearchEntry {
                 cell: {
                     left_attach: 1,
                     top_attach: 0,
                 }
             },
             #[style_class="label"]
             gtk::Label {
                 cell: {
                     left_attach: 0,
                     top_attach: 1,
                 },
                 text: "Details",
                 halign: gtk::Align::End,
             },
             gtk::SearchEntry {
                 cell: {
                     left_attach: 1,
                     top_attach: 1,
                 },
             },
         },
    }
}
