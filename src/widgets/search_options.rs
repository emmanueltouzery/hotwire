use crate::search_expr::SearchOperator;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

pub enum CombineOperator {
    And,
    Or,
}

#[derive(Msg)]
pub enum Msg {
    AddClick,
    AddAndCloseClick,
    Add(
        (
            Option<CombineOperator>,
            &'static str,
            SearchOperator,
            String,
        ),
    ),
    DisableOptions,
    EnableOptionsWithAndOr,
    EnableOptionsWithoutAndOr,
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
            Msg::DisableOptions => {
                self.widgets.root_grid.set_sensitive(false);
            }
            Msg::EnableOptionsWithAndOr => {
                self.widgets.root_grid.set_sensitive(true);
                self.widgets.and_or_combo.set_sensitive(true);
            }
            Msg::EnableOptionsWithoutAndOr => {
                self.widgets.root_grid.set_sensitive(true);
                self.widgets.and_or_combo.set_sensitive(false);
            }
            Msg::AddClick => {}
            Msg::AddAndCloseClick => {}
            // meant for my parent
            Msg::Add(_) => {}
        }
    }

    view! {
        #[name="root_grid"]
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
                     clicked => Msg::AddClick,
                 },
                 gtk::Button {
                     label: "Add and close",
                     clicked => Msg::AddAndCloseClick,
                 },
             },
         },
    }
}
