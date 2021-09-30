use super::search_options::SearchOptions;
use gtk::prelude::*;
use relm::{Component, Widget};
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {
    SearchClicked,
    SearchActiveChanged(bool),
    SearchTextChanged(String),
    SearchTextChangedFromElsewhere((String, gdk::EventKey)),
}

pub struct Model {
    relm: relm::Relm<HeaderbarSearch>,
    search_toggle_signal: Option<glib::SignalHandlerId>,
    search_options: Option<Component<SearchOptions>>,
}

enum Operator {
    And,
    Or,
}

enum FilterElement {
    GridCell(String),
    DetailContents(String),
}

struct SearchInfo {
    elements: Vec<(Operator, FilterElement)>,
}

#[derive(PartialEq, Eq, Debug)]
enum Token<'a> {
    String(&'a str),
    ContainKeyword,
    GridCellsKeyword,
    DetailContentsKeyword,
    AndKeyword,
    OrKeyword,
    InvalidExpr(&'a str),
}

fn parse_search_expression(expr: &str) -> SearchInfo {
    let mut rest = Some(expr);
    let mut grid_cells = None;
    let mut detail_contents = None;
    loop {
        let token_rest = search_expression_next_token(rest.unwrap());
        match token_rest.0 {
            Some(Token::InvalidExpr(_)) => {
                // abort everything, the whole string is a grid filter.
                grid_cells = Some(expr);
                detail_contents = None;
                break;
            }
        }
        rest = token_rest.1;
        if rest.is_none() {
            break;
        }
    }
    SearchInfo {
        grid_cells,
        detail_contents,
    }
}

fn search_expression_next_token<'a>(expr: &'a str) -> (Option<Token<'a>>, Option<&'a str>) {
    let mut offset = 0;
    let mut end_chr = None;
    let mut cur_token_start = 0;
    for chr in expr.chars() {
        match end_chr {
            None => {
                // still searching for the token start
                if chr == ' ' {
                    offset += 1;
                    continue;
                }
                cur_token_start = offset;
                if chr == '"' {
                    end_chr = Some('"');
                    offset += 1;
                } else {
                    end_chr = Some(' ');
                }
            }
            Some('"') if chr == '"' => {
                // hit the token end
                return (
                    Some(Token::String(&expr[(cur_token_start + 1)..(offset - 1)])),
                    Some(&expr[offset..]),
                );
            }
            Some(' ') if chr == ' ' => {
                return (
                    Some(parse_simple_token(&expr[cur_token_start..offset])),
                    Some(&expr[offset..]),
                );
            }
            Some(_) => {
                // character within token
            }
        }
        offset += 1;
    }
    // hit the end of the string

    match end_chr {
        Some(' ') => {
            // when we search for the end character ' ',
            // we also accept EOF to conclude the token
            (
                Some(parse_simple_token(&expr[cur_token_start..offset])),
                None,
            )
        }
        Some('"') => {
            // unterminated string, that's an invalid expression
            (
                Some(Token::InvalidExpr(&expr[cur_token_start..(offset - 1)])),
                None,
            )
        }
        _ => (None, None),
    }
}

fn parse_simple_token<'a>(val: &'a str) -> Token<'a> {
    let token = match val {
        "contain" => Token::ContainKeyword,
        "detail_contents" => Token::DetailContentsKeyword,
        "grid_cells" => Token::GridCellsKeyword,
        "and" => Token::AndKeyword,
        "or" => Token::OrKeyword,
        _ => Token::InvalidExpr(val),
    };
    token
}

#[widget]
impl Widget for HeaderbarSearch {
    fn init_view(&mut self) {
        let relm = self.model.relm.clone();
        self.model.search_toggle_signal =
            Some(self.widgets.search_toggle.connect_toggled(move |_| {
                relm.stream().emit(Msg::SearchClicked);
            }));

        self.model.search_options =
            Some(relm::init::<SearchOptions>(()).expect("Error initializing the search options"));

        let search_options_popover = gtk::PopoverBuilder::new()
            .child(self.model.search_options.as_ref().unwrap().widget())
            .build();
        self.widgets
            .search_options_btn
            .set_popover(Some(&search_options_popover));
    }

    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {
            relm: relm.clone(),
            search_toggle_signal: None,
            search_options: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SearchClicked => {
                let new_visible = self.widgets.search_toggle.is_active();
                self.widgets.search_entry.grab_focus();
                self.model
                    .relm
                    .stream()
                    .emit(Msg::SearchActiveChanged(new_visible));
            }
            Msg::SearchActiveChanged(is_active) => {
                self.widgets.search_toggle.set_active(is_active);
                self.widgets.search_box.set_visible(is_active);
            }
            Msg::SearchTextChanged(_) => {} // meant for my parent
            Msg::SearchTextChangedFromElsewhere((txt, _evt)) => {
                if !self.widgets.search_toggle.is_active() {
                    // we want to block the signal of the search button toggle,
                    // because when you click the search button we set the focus
                    // and select the search text. if we did that when search
                    // is triggered by someone typing, the first letter would
                    // be lost when typing the second letter, due to the selection
                    // so we block the search button toggle signal & handle things
                    // by hand.
                    self.widgets
                        .search_toggle
                        .block_signal(self.model.search_toggle_signal.as_ref().unwrap());
                    self.widgets.search_box.set_visible(true);
                    self.widgets.search_toggle.set_active(true);
                    self.widgets.search_entry.grab_focus_without_selecting();

                    self.widgets.search_entry.set_text(&txt);
                    self.widgets
                        .search_toggle
                        .unblock_signal(self.model.search_toggle_signal.as_ref().unwrap());
                    self.widgets.search_entry.set_position(1);
                }
            }
        }
    }

    fn parse_search_entry(&self) -> SearchInfo {
        let search_contents = self.widgets.search_entry.text().to_string();
        if search_contents.contains(" grid_cells ") || search_contents.contains(" details ") {
            parse_search_expression(&search_contents)
        } else {
            SearchInfo {
                grid_cells: Some(search_contents),
                detail_contents: None,
            }
        }
    }

    view! {
        gtk::Box {
            #[style_class="linked"]
            #[name="search_box"]
            gtk::Box {
                visible: false,
                #[name="search_entry"]
                gtk::SearchEntry {
                    changed(entry) => Msg::SearchTextChanged(entry.text().to_string())
                },
                #[name="search_options_btn"]
                gtk::MenuButton {
                    image: Some(&gtk::Image::from_icon_name(Some("document-properties-symbolic"), gtk::IconSize::Menu)),
                    active: false,
                },
            },
            #[name="search_toggle"]
            gtk::ToggleButton {
                image: Some(&gtk::Image::from_icon_name(Some("edit-find-symbolic"), gtk::IconSize::Menu)),
                margin_start: 10,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_tokens_vec(expr: &str) -> Vec<Token> {
        let mut res = vec![];
        let mut rest = Some(expr);
        loop {
            let token_rest = search_expression_next_token(rest.unwrap());
            if token_rest.0.is_some() {
                res.push(token_rest.0.unwrap());
            }
            rest = token_rest.1;
            if rest.is_none() {
                return res;
            }
        }
    }

    #[test]
    fn tokenize_empty_search_expression() {
        assert_eq!(Vec::<Token>::new(), get_tokens_vec(""));
    }

    #[test]
    fn tokenize_incomplete_string_search_expression() {
        assert_eq!(vec![Token::InvalidExpr("\"a")], get_tokens_vec("\"a"));
    }

    #[test]
    fn tokenize_simple_search_expression() {
        assert_eq!(
            vec![
                Token::GridCellsKeyword,
                Token::ContainKeyword,
                Token::String("test")
            ],
            get_tokens_vec("grid_cells contain \"test\"")
        );
    }

    #[test]
    fn tokenize_combined_search_expression() {
        assert_eq!(
            vec![
                Token::GridCellsKeyword,
                Token::ContainKeyword,
                Token::String("test"),
                Token::AndKeyword,
                Token::DetailContentsKeyword,
                Token::ContainKeyword,
                Token::String("details val"),
            ],
            get_tokens_vec(
                "grid_cells contain \"test\" and detail_contents contain \"details val\""
            )
        );
    }
}
