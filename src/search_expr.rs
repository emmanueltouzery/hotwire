use nom;
use nom::branch::*;
use nom::bytes::complete::tag;
use nom::character::complete::*;
use nom::combinator::*;
use nom::error::*;
use nom::multi::*;
use nom::sequence::*;
use nom::AsChar;
use nom::Err;
use std::collections::HashSet;
use std::fmt;

#[derive(PartialEq, Eq)]
pub struct SearchOpExpr {
    pub filter_key: &'static str,
    pub op: SearchOperator,
    pub op_negation: OperatorNegation,
    pub filter_val: String,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum OperatorNegation {
    Negated,
    NotNegated,
}

#[derive(PartialEq, Eq)]
pub enum SearchExpr {
    And(Box<SearchExpr>, Box<SearchExpr>),
    Or(Box<SearchExpr>, Box<SearchExpr>),
    SearchOpExpr(SearchOpExpr),
}

fn print_parent(f: &mut fmt::Formatter<'_>, depth: i32, title: &str) -> Result<(), fmt::Error> {
    for _ in 0..depth {
        f.write_str(" ")?;
    }
    f.write_fmt(format_args!("{}\n", title))?;
    Ok(())
}

fn print_node(f: &mut fmt::Formatter<'_>, depth: i32, node: &SearchExpr) -> Result<(), fmt::Error> {
    match node {
        SearchExpr::And(lhs, rhs) => {
            print_parent(f, depth, "and")?;
            print_node(f, depth + 1, lhs)?;
            print_node(f, depth + 1, rhs)?;
        }
        SearchExpr::Or(lhs, rhs) => {
            print_parent(f, depth, "or")?;
            print_node(f, depth + 1, lhs)?;
            print_node(f, depth + 1, rhs)?;
        }
        SearchExpr::SearchOpExpr(SearchOpExpr {
            filter_key,
            op,
            op_negation,
            filter_val,
        }) => {
            print_parent(f, depth, &format!("{:?} {:?}", op, op_negation))?;
            print_parent(f, depth + 1, filter_key)?;
            print_parent(f, depth + 1, filter_val)?;
        }
    }
    Ok(())
}

impl fmt::Debug for SearchExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str("\n")?;
        print_node(f, 0, self)
    }
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum SearchOperator {
    Contains,
}

pub fn parse_search<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + Fn(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |input: &'a str| {
        alt((
            parse_search_and(known_filter_keys),
            parse_search_or(known_filter_keys),
            delimited(
                with_spaces_ba(tag("(")),
                parse_search(known_filter_keys),
                with_spaces_b(tag(")")),
            ),
            parse_search_expr(known_filter_keys),
        ))(input)
    }
}

// b = before
fn with_spaces_b<'a, P>(p: P) -> impl FnMut(&'a str) -> nom::IResult<&'a str, &'a str>
where
    P: Fn(&'a str) -> nom::IResult<&'a str, &'a str>,
{
    move |input: &str| {
        let (input, _) = space0(input)?;
        let (input, r) = p(input)?;
        Ok((input, r))
    }
}

// ba = before/after
fn with_spaces_ba<'a, P>(p: P) -> impl FnMut(&'a str) -> nom::IResult<&'a str, &'a str>
where
    P: Fn(&'a str) -> nom::IResult<&'a str, &'a str>,
{
    move |input: &str| {
        let (input, _) = space0(input)?;
        let (input, r) = p(input)?;
        let (input, _) = space0(input)?;
        Ok((input, r))
    }
}

fn parse_search_and<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + FnMut(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |input: &str| {
        let (input, se) = alt((
            parse_search_expr(known_filter_keys),
            delimited(
                with_spaces_ba(tag("(")),
                parse_search(known_filter_keys),
                with_spaces_b(tag(")")),
            ),
        ))(input)?;
        let (input, _) = space1(input)?;
        let (input, _) = tag("and")(input)?;
        let (input, _) = space1(input)?;
        let next_is_bracketed =
            peek::<_, _, nom::error::Error<&str>, _>(with_spaces_ba(tag("(")))(input).is_ok();
        let (input, se2) = parse_search(known_filter_keys)(input)?;
        match se2 {
            // we want AND to bind tighter than OR
            // so not...
            // a AND (b OR c)
            // but rather...
            // (a AND b) OR c
            // at the same time we don't want to reorder if the next expression is
            // bracketed, for instance "a and (b or c)"
            SearchExpr::Or(ose1, ose2) if !next_is_bracketed => Ok((
                input,
                SearchExpr::Or(Box::new(SearchExpr::And(Box::new(se), ose1)), ose2),
            )),
            _ => Ok((input, SearchExpr::And(Box::new(se), Box::new(se2)))),
        }
    }
}

fn parse_search_or<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + Fn(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |input: &str| {
        let (input, se) = alt((
            parse_search_expr(known_filter_keys),
            delimited(
                with_spaces_ba(tag("(")),
                parse_search(known_filter_keys),
                with_spaces_b(tag(")")),
            ),
        ))(input)?;
        let (input, _) = space1(input)?;
        let (input, _) = tag("or")(input)?;
        let (input, _) = space1(input)?;
        let (input, se2) = parse_search(known_filter_keys)(input)?;
        Ok((input, SearchExpr::Or(Box::new(se), Box::new(se2))))
    }
}

// TODO allow negation (not X contains Y)
fn parse_search_expr<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + Fn(&str) -> nom::IResult<&str, SearchExpr> {
    move |input: &str| {
        let (input, filter_key) = parse_filter_key(known_filter_keys.clone())(input)?;
        let (input, _) = space1(input)?;
        let (input, (op, op_negation)) = parse_filter_op(input)?;
        let (input, _) = space1(input)?;
        let (input, filter_val) = parse_filter_val(input)?;
        Ok((
            input,
            SearchExpr::SearchOpExpr(SearchOpExpr {
                filter_key,
                op,
                op_negation,
                filter_val,
            }),
        ))
    }
}

fn parse_filter_val(input: &str) -> nom::IResult<&str, String> {
    alt((parse_quoted_string, parse_word))(input)
}

fn parse_filter_op(input: &str) -> nom::IResult<&str, (SearchOperator, OperatorNegation)> {
    let (input, t) = alt((tag("doesntContain"), tag("contains")))(input)?;
    match t {
        "contains" => Ok((
            input,
            (SearchOperator::Contains, OperatorNegation::NotNegated),
        )),
        "doesntContain" => Ok((input, (SearchOperator::Contains, OperatorNegation::Negated))),
        _ => panic!("unhandled: {}", t),
    }
}

fn parse_filter_key(
    known_filter_keys: HashSet<&'static str>,
) -> impl Fn(&str) -> nom::IResult<&str, &'static str> {
    move |input: &str| {
        // let (input, filter_key) = recognize(parse_filter_key_basic)(input)?;
        map_res(recognize(parse_filter_key_basic), |s: &str| {
            known_filter_keys
                .get(s)
                .map(|s| *s)
                .ok_or(Err::Error(("bad filter key", ErrorKind::Verify)))
        })(input)
    }
}

fn parse_filter_key_basic(input: &str) -> nom::IResult<&str, ()> {
    let (input, _) = alpha1(input)?;
    let (input, _) = char('.')(input)?;
    let (input, _) = many1(satisfy(|c| c.is_alpha() || c == '_'))(input)?;
    Ok((input, ()))
}

fn parse_quoted_string(input: &str) -> nom::IResult<&str, String> {
    let (input, _) = char('"')(input)?;
    let (input, st) = fold_many0(quoted_string_char, String::new, |mut sofar, cur| {
        sofar.push(cur);
        sofar
    })(input)?;
    let (input, _) = char('"')(input)?;
    Ok((input, st))
}

fn quoted_string_char(input: &str) -> nom::IResult<&str, char> {
    alt((none_of("\\\""), escaped_char))(input)
}

// meant for \" mostly for now
fn escaped_char(input: &str) -> nom::IResult<&str, char> {
    let (input, _) = char('\\')(input)?;
    none_of("\\")(input)
}

fn parse_word(input: &str) -> nom::IResult<&str, String> {
    // i want to allow unicode characters here, so alphanumeric is not good enough, I think
    fold_many1(
        none_of(" \t\r\n()\",;:!?/*+"),
        String::new,
        |mut sofar, cur| {
            sofar.push(cur);
            sofar
        },
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quoted_string_simple_case() {
        assert_eq!(
            "my \"string",
            parse_quoted_string("\"my \\\"string\"").unwrap().1
        );
    }

    #[test]
    fn should_reject_unknown_filter_key() {
        assert_eq!(
            true,
            parse_search(&["detail.contents"].iter().cloned().collect())(
                "grid.cells contains test"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_combined_search_expression() {
        assert_eq!(
            (
                "",
                (SearchExpr::Or(
                    Box::new(SearchExpr::And(
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "grid.cells",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "test".to_string(),
                        })),
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "details val".to_string(),
                        })),
                    )),
                    Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                        filter_key: "detail.contents",
                        op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                        filter_val: "val2".to_string(),
                    })),
                ))
            ),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("grid.cells contains test and detail.contents contains \"details val\" or detail.contents contains val2")
            .unwrap()
        );
    }

    #[test]
    fn parse_combined_search_expression_with_brackets() {
        assert_eq!(
            Ok((
                "",
                (SearchExpr::And(
                    Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                        filter_key: "grid.cells",
                        op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                        filter_val: "test".to_string(),
                    })),
                    Box::new(SearchExpr::Or(
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "details val".to_string(),
                        })),
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::Negated,
                            filter_val: "val2".to_string(),
                        })),
                    ))
                ))
            )),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )(
                "grid.cells contains test and (detail.contents contains \"details val\" or detail.contents doesntContain val2)"
            )
        );
    }

    #[test]
    fn parse_combined_search_expression_with_brackets2() {
        assert_eq!(
            (
                "",
                (SearchExpr::Or(
                    Box::new(SearchExpr::And(
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "grid.cells",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "test".to_string(),
                        })),
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "details val".to_string(),
                        })),
                    )),
                    Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                        filter_key: "detail.contents",
                        op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                        filter_val: "val2".to_string(),
                    })),
                ))
            ),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("(grid.cells contains test and detail.contents contains \"details val\") or detail.contents contains val2")
            .unwrap()
        );
    }

    #[test]
    fn parse_combined_search_expression_with_brackets3() {
        assert_eq!(
            (
                "",
                (SearchExpr::Or(Box::new(SearchExpr::And(
                    Box::new(SearchExpr::And(
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "grid.cells",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "test".to_string(),
                        })),
                        Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            op_negation: OperatorNegation::NotNegated,
                            filter_val: "details val".to_string(),
                        })),
                    )),
                    Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                        filter_key: "detail.contents",
                        op: SearchOperator::Contains,
                        op_negation: OperatorNegation::NotNegated,
                        filter_val: "val2".to_string(),
                    })),
                )),
                 Box::new(SearchExpr::SearchOpExpr(SearchOpExpr {
                     filter_key: "grid.cells",
                     op: SearchOperator::Contains,
                     op_negation: OperatorNegation::NotNegated,
                     filter_val: "val3".to_string(),
                 }))
            ))),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("(grid.cells contains test and detail.contents contains \"details val\") and detail.contents contains val2 or grid.cells contains val3")
            .unwrap()
        );
    }
}
