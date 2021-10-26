use super::search_options;
use super::search_options::Msg as SearchOptionsMsg;
use super::search_options::SearchOptions;
use crate::search_expr;
use crate::search_expr::SearchExpr;
use gtk::prelude::EntryCompletionExtManual;
use gtk::prelude::*;
use relm::{Component, Widget};
use relm_derive::{widget, Msg};
use std::collections::{BTreeMap, BTreeSet};

const ITEM_TYPE_FILTER_KEY: u32 = 0;
const ITEM_TYPE_SEARCH_OPERATOR: u32 = 1;
const ITEM_TYPE_FILTER_COMBINATOR: u32 = 2;

#[derive(Msg)]
pub enum Msg {
    SearchActiveChanged(bool),
    MainWinSelectCard(usize),
    SearchTextChanged(String),
    SearchExprChanged(Option<Result<(String, search_expr::SearchExpr), String>>),
    SearchFilterKeysChanged(BTreeSet<&'static str>),
    ClearSearchTextClick,
    DisplayNoSearchError,
    DisplayWithSearchErrors,
    OpenSearchAddPopover,
    SearchAddVals(
        (
            Option<search_options::CombineOperator>,
            &'static str,
            search_expr::SearchOperator,
            search_expr::OperatorNegation,
            String,
        ),
    ),
    RequestOptionsClose,
    SearchCompletionAction(String),
}

pub struct Model {
    relm: relm::Relm<HeaderbarSearch>,
    search_options: Option<Component<SearchOptions>>,
    // store the filter keys in a BTreeSet so that they'll be sorted
    // alphabetically when displaying in the GUI. An in a set because we
    // need to test for 'contains' a couple of times
    known_filter_keys: BTreeSet<&'static str>,
    current_card_idx: Option<usize>,
    search_text_by_card: BTreeMap<usize, String>,
    search_completion: gtk::EntryCompletion,
}

#[widget]
impl Widget for HeaderbarSearch {
    fn init_view(&mut self) {
        let relm = self.model.relm.clone();

        let completion = &self.model.search_completion;
        let r = relm.clone();
        completion.connect_match_selected(move |compl, model, iter| {
            let chosen_completion = model.value(iter, 0).get::<String>().unwrap();
            r.stream()
                .emit(Msg::SearchCompletionAction(chosen_completion));
            gtk::Inhibit(true)
        });
        self.widgets.search_entry.set_completion(Some(completion));

        let so = relm::init::<SearchOptions>(self.model.known_filter_keys.clone())
            .expect("Error initializing the search options");
        relm::connect!(so@SearchOptionsMsg::Add(ref vals), self.model.relm, Msg::SearchAddVals(vals.clone()));
        relm::connect!(so@SearchOptionsMsg::AddAndCloseClick, self.model.relm, Msg::RequestOptionsClose);
        relm::connect!(so@SearchOptionsMsg::ClearSearchTextClick, self.model.relm, Msg::ClearSearchTextClick);
        self.model.search_options = Some(so);

        let search_options_popover = gtk::PopoverBuilder::new()
            .child(self.model.search_options.as_ref().unwrap().widget())
            .build();
        self.model
            .search_options
            .as_ref()
            .unwrap()
            .stream()
            .emit(SearchOptionsMsg::ParentSet(search_options_popover.clone()));
        self.widgets
            .search_options_btn
            .set_popover(Some(&search_options_popover));
        self.update_search_status(None);
    }

    fn model(relm: &relm::Relm<Self>, known_filter_keys: BTreeSet<&'static str>) -> Model {
        let cell_area = gtk::CellAreaBoxBuilder::new().build();
        let text_cell_type = gtk::CellRendererTextBuilder::new()
            // from the gnome color palette https://developer.gnome.org/hig/reference/palette.html?highlight=color
            .background("#f8e45c")
            .style(pango::Style::Italic)
            .build();
        let text_cell_main = gtk::CellRendererTextBuilder::new().build();
        CellAreaBoxExt::pack_start(&cell_area, &text_cell_type, false, true, true);
        CellAreaBoxExt::pack_start(&cell_area, &text_cell_main, true, true, true);
        let search_completion = gtk::EntryCompletionBuilder::new()
            .cell_area(&cell_area)
            .text_column(0)
            .build();
        search_completion.add_attribute(&text_cell_type, "text", 2);
        search_completion.add_attribute(&text_cell_main, "text", 0);
        Model {
            relm: relm.clone(),
            search_options: None,
            known_filter_keys,
            current_card_idx: None,
            search_text_by_card: BTreeMap::new(),
            search_completion,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SearchFilterKeysChanged(hash) => {
                if let Some(so) = self.model.search_options.as_ref() {
                    so.stream()
                        .emit(search_options::Msg::FilterKeysUpdated(hash.clone()));
                }
                self.update_search_completion(&hash);
                self.model.known_filter_keys = hash;
            }
            Msg::SearchActiveChanged(is_active) => {
                if is_active {
                    self.widgets.search_entry.grab_focus();
                }
            }
            Msg::MainWinSelectCard(idx) => {
                if let Some(cur_idx) = self.model.current_card_idx {
                    // backup the current text
                    self.model
                        .search_text_by_card
                        .insert(cur_idx, self.widgets.search_entry.text().to_string());
                }
                if let Some(txt) = self.model.search_text_by_card.get(&idx) {
                    self.widgets.search_entry.set_text(txt);
                } else {
                    self.widgets.search_entry.set_text("");
                }
                self.model.current_card_idx = Some(idx);
            }
            Msg::SearchTextChanged(text) => {
                let maybe_expr = if text.is_empty() {
                    None
                } else {
                    Some(
                        search_expr::parse_search(&self.model.known_filter_keys)(&text)
                            .map(|(rest, expr)| (rest.to_string(), expr))
                            .map_err(|e| e.to_string()),
                    )
                };
                self.model
                    .relm
                    .stream()
                    .emit(Msg::SearchExprChanged(maybe_expr));
            }
            Msg::SearchCompletionAction(completion) => {
                let updated_text = Self::search_completion_action(
                    &self.widgets.search_entry.text().to_string(),
                    self.widgets.search_entry.cursor_position() as usize,
                    completion,
                );
                self.widgets.search_entry.set_text(&updated_text);
                self.widgets.search_entry.set_position(-1);
            }
            Msg::SearchExprChanged(expr) => {
                self.update_search_status(expr);
            }
            Msg::DisplayNoSearchError => {
                self.widgets
                    .search_entry
                    .set_primary_icon_name(Some("edit-find-symbolic"));
                self.widgets
                    .search_entry
                    .set_primary_icon_tooltip_text(None);
            }
            Msg::DisplayWithSearchErrors => {
                self.widgets
                    .search_entry
                    .set_primary_icon_name(Some("computer-fail-symbolic"));
                self.widgets
                    .search_entry
                    .set_primary_icon_tooltip_text(Some("Invalid search expression"));
            }
            Msg::SearchAddVals((combine_op, filter_key, search_op, op_negation, val)) => {
                let mut t = self.widgets.search_entry.text().to_string();
                match combine_op {
                    Some(search_options::CombineOperator::And) => {
                        t.push_str(" and ");
                    }
                    Some(search_options::CombineOperator::Or) => {
                        t.push_str(" or ");
                    }
                    None => {}
                }
                t.push_str(filter_key);
                match (search_op, op_negation) {
                    (
                        search_expr::SearchOperator::Contains,
                        search_expr::OperatorNegation::NotNegated,
                    ) => {
                        t.push_str(" contains ");
                    }
                    (
                        search_expr::SearchOperator::Contains,
                        search_expr::OperatorNegation::Negated,
                    ) => {
                        t.push_str(" doesntContain ");
                    }
                }
                if val.contains(' ') {
                    t.push('"');
                    t.push_str(&val.replace('"', "\""));
                    t.push('"');
                } else {
                    t.push_str(&val);
                }
                self.widgets.search_entry.set_text(&t);
            }
            Msg::ClearSearchTextClick => {
                self.widgets.search_entry.set_text("");
            }
            Msg::RequestOptionsClose => {
                if let Some(popover) = self.widgets.search_options_btn.popover() {
                    popover.popdown();
                }
            }
            Msg::OpenSearchAddPopover => {
                if let Some(popover) = self.widgets.search_options_btn.popover() {
                    popover.popup();
                }
            }
        }
    }

    fn search_completion_action(
        entry_text_before: &str,
        entry_pos: usize,
        completion: String,
    ) -> String {
        let txt = &entry_text_before[0..entry_pos];
        let rest = if entry_pos < entry_text_before.len() {
            &entry_text_before[entry_pos..]
        } else {
            " "
        };
        if txt.contains(' ') {
            let base = txt
                .rsplitn(2, |c| c == ' ')
                .last()
                .unwrap_or("")
                .to_string();
            base + " " + &completion + rest
        } else {
            completion + rest
        }
    }

    fn update_search_completion(&mut self, known_filter_keywords: &BTreeSet<&'static str>) {
        let store = gtk::ListStore::new(&[
            String::static_type(), // completion
            u32::static_type(),    // item type
            String::static_type(), // item type display
        ]);
        store.insert_with_values(
            None,
            &[
                (0, &"and".to_value()),
                (1, &ITEM_TYPE_SEARCH_OPERATOR.to_value()),
                (2, &"Search operator".to_value()),
            ],
        );
        store.insert_with_values(
            None,
            &[
                (0, &"or".to_value()),
                (1, &ITEM_TYPE_SEARCH_OPERATOR.to_value()),
                (2, &"Search operator".to_value()),
            ],
        );
        for keyword in known_filter_keywords {
            store.insert_with_values(
                None,
                &[
                    (0, &keyword.to_value()),
                    (1, &ITEM_TYPE_FILTER_KEY.to_value()),
                    (2, &"Filter key".to_value()),
                ],
            );
        }
        // TODO duplicated with search_expr::parse_filter_op
        store.insert_with_values(
            None,
            &[
                (0, &"contains".to_value()),
                (1, &ITEM_TYPE_FILTER_COMBINATOR.to_value()),
                (2, &"Filter combinator".to_value()),
            ],
        );
        store.insert_with_values(
            None,
            &[
                (0, &"doesntContain".to_value()),
                (1, &ITEM_TYPE_FILTER_COMBINATOR.to_value()),
                (2, &"Filter combinator".to_value()),
            ],
        );
        self.model.search_completion.set_model(Some(&store));
        self.model
            .search_completion
            .set_match_func(|compl, full_txt, iter| {
                let e = compl.entry().unwrap();
                let txt = &full_txt[..(e.cursor_position() as usize)];
                let last_typed_word = txt.split(' ').last().unwrap_or(txt);
                let possible_completion_txt = compl
                    .model()
                    .unwrap()
                    .value(iter, 0)
                    .get::<String>()
                    .unwrap();
                possible_completion_txt.starts_with(last_typed_word)
            });
    }

    fn update_search_status(&mut self, maybe_expr: Option<Result<(String, SearchExpr), String>>) {
        if let Some(opt) = self.model.search_options.as_ref() {
            match maybe_expr {
                None => {
                    self.model.relm.stream().emit(Msg::DisplayNoSearchError);
                    opt.stream()
                        .emit(search_options::Msg::EnableOptionsWithoutAndOr);
                }
                Some(Ok((rest, _))) if rest.is_empty() => {
                    self.model.relm.stream().emit(Msg::DisplayNoSearchError);
                    opt.stream()
                        .emit(search_options::Msg::EnableOptionsWithAndOr);
                }
                _ => {
                    self.model.relm.stream().emit(Msg::DisplayWithSearchErrors);
                    opt.stream().emit(search_options::Msg::DisableOptions);
                }
            }
        }
    }

    view! {
        gtk::Box {
            #[style_class="linked"]
            #[name="search_box"]
            gtk::Box {
                #[name="search_entry"]
                gtk::SearchEntry {
                    hexpand: true,
                    secondary_icon_name: Some("edit-clear-symbolic"),
                    placeholder_text: Some("Enter a filter expression. Help yourself with the 'Add filter criteria' button to get started."),
                    changed(entry) => Msg::SearchTextChanged(entry.text().to_string()),
                },
                #[name="search_options_btn"]
                gtk::MenuButton {
                    image: Some(&gtk::Image::from_icon_name(Some("insert-symbolic"), gtk::IconSize::Menu)),
                    always_show_image: true,
                    label: "Add filter criteria",
                    active: false,
                },
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_first_word() {
        assert_eq!(
            "http.text ",
            HeaderbarSearch::search_completion_action("http.t", 6, "http.text".to_string())
        );
    }

    #[test]
    fn completion_at_end() {
        assert_eq!(
            "http.text contains ",
            HeaderbarSearch::search_completion_action("http.text con", 13, "contains".to_string())
        );
    }

    #[test]
    fn completion_in_middle() {
        assert_eq!(
            "http.text contains",
            HeaderbarSearch::search_completion_action("ht contains", 2, "http.text".to_string())
        );
    }
}
