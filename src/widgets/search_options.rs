use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg, Debug)]
pub enum Msg {
    Add,
    AddAndClose,
}

pub struct Model {}

#[widget]
impl Widget for SearchOptions {
    fn init_view(&mut self) {
        self.widgets.and_or_combo.append_text("and");
        self.widgets.and_or_combo.append_text("or");
        self.widgets.and_or_combo.set_active(Some(0));
        self.widgets.filter_key_combo.append_text("grid.cells");
        self.widgets.filter_key_combo.append_text("detail.contents");
        self.widgets.filter_key_combo.set_active(Some(0));
        self.widgets.search_op_combo.append_text("contains");
        self.widgets.search_op_combo.set_active(Some(0));
    }
    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {}
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::Add => {}
            Msg::AddAndClose => {}
        }
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
             #[name="and_or_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 0,
                     top_attach: 0,
                 },
             },
             #[name="filter_key_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 1,
                     top_attach: 0,
                 },
             },
             #[name="search_op_combo"]
             gtk::ComboBoxText {
                 cell: {
                     left_attach: 0,
                     top_attach: 1,
                 },
             },
             gtk::SearchEntry {
                 cell: {
                     left_attach: 1,
                     top_attach: 1,
                 },
             },
             gtk::ButtonBox {
                 cell: {
                     left_attach: 0,
                     top_attach: 2,
                     width: 2,
                 },
                 layout_style: gtk::ButtonBoxStyle::Expand,
                 gtk::Button {
                     label: "Add",
                     clicked => Msg::Add,
                 },
                 gtk::Button {
                     label: "Add and close",
                     clicked => Msg::AddAndClose,
                 },
             },
         },
    }
}
