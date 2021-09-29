use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {}

pub struct Model {}

#[widget]
impl Widget for SearchOptions {
    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {}
    }

    fn update(&mut self, event: Msg) {}

    view! {
         gtk::Box {
             orientation: gtk::Orientation::Horizontal,
             margin_top: 10,
             margin_start: 10,
             margin_end: 10,
             margin_bottom: 10,
             spacing: 10,
             gtk::Switch {},
             gtk::Label {
                 text: "Advanced search mode"
             }
         },
    }
}
