use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Clone)]
pub struct PostgresMessageData {
    pub query: String,
}

#[derive(Msg)]
pub enum Msg {}

pub struct Model {
    data: PostgresMessageData,
}

#[widget]
impl Widget for PostgresCommEntry {
    fn model(relm: &relm::Relm<Self>, data: PostgresMessageData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            gtk::Separator {},
            gtk::Label {
                label: &self.model.data.query,
                xalign: 0.0
            },
        }
    }
}
