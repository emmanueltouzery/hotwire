pub const BODY_TRUNCATE_LIMIT_BYTES: usize = 128 * 1024;

pub fn highlight_indent_truncate(
    do_format: bool,
    body: &str,
    content_type: Option<&str>,
) -> String {
    // support eg "application/xml;charset=UTF8"
    let content_type_first_part = content_type.map(|c| {
        if let Some(semicolon_index) = c.find(';') {
            &c[0..semicolon_index]
        } else {
            c
        }
    });
    let truncated_body = if body.len() > BODY_TRUNCATE_LIMIT_BYTES {
        &body[0..BODY_TRUNCATE_LIMIT_BYTES]
    } else {
        body
    };
    match content_type_first_part {
        Some("application/xml") | Some("text/xml") if do_format => {
            highlight_indent_xml(truncated_body)
        }
        Some("application/json") | Some("text/json") if do_format => {
            highlight_indent_json(truncated_body)
        }
        _ => glib::markup_escape_text(truncated_body).to_string(),
    }
}

fn highlight_indent_xml(xml: &str) -> String {
    let mut indent = 0;
    let mut result = "".to_string();
    let mut has_attributes = false;
    let mut has_text = false;
    let mut attrs_on_line = 0;
    for token in xmlparser::Tokenizer::from(xml) {
        // dbg!(token);
        match token {
            Ok(xmlparser::Token::ElementStart { local, .. }) => {
                if result.len() > 0 {
                    result.push('\n');
                    for _ in 0..indent {
                        result.push_str("  ");
                    }
                }
                result.push_str("&lt;<b>");
                result.push_str(&local);
                has_attributes = false;
                has_text = false;
                attrs_on_line = 0;
            }
            Ok(xmlparser::Token::Attribute { span, .. }) => {
                if !has_attributes {
                    result.push_str("</b>");
                }
                attrs_on_line += 1;
                if attrs_on_line > 3 {
                    result.push('\n');
                    for _ in 0..(indent + 1) {
                        result.push_str("  ");
                    }
                    attrs_on_line = 0;
                }
                result.push(' ');
                result.push_str(&span);
                has_attributes = true;
            }
            Ok(xmlparser::Token::ElementEnd {
                end: xmlparser::ElementEnd::Open,
                ..
            }) => {
                // ">"
                if has_attributes {
                    result.push_str("&gt;");
                } else {
                    result.push_str("</b>&gt;");
                }
                indent += 1;
                has_text = false;
            }
            Ok(xmlparser::Token::ElementEnd {
                end: xmlparser::ElementEnd::Empty,
                ..
            }) =>
            // "/>"
            {
                if has_attributes {
                    result.push_str("/&gt;");
                } else {
                    result.push_str("</b>/&gt;");
                }
            }
            Ok(xmlparser::Token::ElementEnd {
                end: xmlparser::ElementEnd::Close(_, name),
                ..
            }) => {
                // </name>
                indent -= 1;
                if !has_text {
                    result.push('\n');
                    for _ in 0..indent {
                        result.push_str("  ");
                    }
                }
                result.push_str("&lt;/<b>");
                result.push_str(&name);
                result.push_str("</b>&gt;");
                has_text = false;
            }
            Ok(xmlparser::Token::Text { text, .. }) => {
                let txt = text.replace("\n", "").trim().to_string();
                if !txt.is_empty() {
                    result.push_str(&txt);
                    has_text = true;
                }
            }
            Ok(xmlparser::Token::Declaration { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::ProcessingInstruction { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::Comment { text, .. }) => {
                result.push_str(" <i>&lt;!-- ");
                result.push_str(&text);
                result.push_str(" --&gt;</i>");
            }
            Ok(xmlparser::Token::DtdStart { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::EmptyDtd { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::DtdEnd { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::EntityDeclaration { span, .. }) => {
                result.push_str(&span);
            }
            Ok(xmlparser::Token::Cdata { span, .. }) => {
                result.push_str(&span);
            }
            Err(_) => return xml.to_string(),
        }
    }
    result
}

fn highlight_indent_json(json: &str) -> String {
    if let Ok(val) = serde_json::from_str(json) {
        highlight_indent_json_value(&val, 0)
    } else {
        json.to_string()
    }
}

fn highlight_indent_json_value(v: &serde_json::Value, indent_depth: usize) -> String {
    let next_indent = " ".repeat((indent_depth + 1) * 2);
    let cur_indent = &next_indent[0..(next_indent.len() - 2)];
    match v {
        serde_json::Value::Object(fields) => {
            "{".to_string()
                + &fields
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\n{}\"<b>{}</b>\": {}",
                            next_indent,
                            k,
                            highlight_indent_json_value(v, indent_depth + 1)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
                + "\n"
                + cur_indent
                + "}"
        }
        serde_json::Value::Array(entries) if entries.is_empty() => "[]".to_string(),
        serde_json::Value::Array(entries) => {
            "[".to_string()
                + &entries
                    .iter()
                    .map(|e| {
                        format!(
                            "\n{}{}",
                            &next_indent,
                            highlight_indent_json_value(e, indent_depth + 1)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
                + "\n"
                + cur_indent
                + "]"
        }
        _ => v.to_string(),
    }
}

#[test]
fn simple_xml_indent() {
    assert_eq!(
    "<?xml?>\n&lt;<b>body</b>&gt;\n  &lt;<b>tag1</b>/&gt;\n  &lt;<b>tag2</b> attr=\"val\"&gt;contents&lt;/<b>tag2</b>&gt;\n&lt;/<b>body</b>&gt;",
    highlight_indent_xml("<?xml?><body><tag1/><tag2 attr=\"val\">contents</tag2></body>")
);
}

#[test]
fn simple_xml_indent_already_indented() {
    assert_eq!(
    "<?xml?>\n&lt;<b>body</b>&gt;\n  &lt;<b>tag1</b>/&gt;\n  &lt;<b>tag2</b> attr=\"val\"&gt;contents&lt;/<b>tag2</b>&gt;\n&lt;/<b>body</b>&gt;",
    highlight_indent_xml("<?xml?>\n<body>\n\n\n      <tag1/>\n\n\n<tag2 attr=\"val\">contents</tag2>\n</body>")
);
}

#[test]
fn xml_highlight_attrs_no_children() {
    assert_eq!(
        "&lt;<b>mytag</b> attr1=\"a\" attr2=\"b\"/&gt;",
        highlight_indent_xml("<mytag attr1=\"a\" attr2=\"b\" />")
    );
}

#[test]
fn xml_indent_long_lines() {
    assert_eq!(
    "&lt;<b>mytag</b> firstattr=\"first value\" secondattr=\"second value\" thirdattr=\"third value\"\n   fourthattr=\"fourth value\" fifthattr=\"fifth value\"/&gt;",
    highlight_indent_xml("<mytag firstattr=\"first value\" secondattr=\"second value\" thirdattr=\"third value\" fourthattr=\"fourth value\" fifthattr=\"fifth value\"/>"))
}

#[test]
fn simple_json_indent() {
    assert_eq!(
        "{\n  \"<b>field1</b>\": 12,\n  \"<b>field2</b>\": [\n    \"hi\",\n    \"array\"\n  ]\n}",
        highlight_indent_json(r#"{"field1": 12, "field2": ["hi", "array"]}"#)
    );
}
