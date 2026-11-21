//! Open questions that were already answered (#177): a section that opens with
//! a None statement has no questions, whatever follows it.

use xherd_flow::questions::parse;

#[test]
fn none_then_answered_items_is_no_question() {
    let md = "## Open questions\nNone. Answered at the first review:\n\
              - Q1 (live or restart): restart needed (R7).\n\
              - Q2 (build steps): refused for now (R5a).\n\n## Assumptions\n- a\n";
    assert_eq!(parse(md), Vec::<String>::new());
}

#[test]
fn none_item_then_more_items_is_no_question() {
    let md = "## Open questions\n- None.\n- Q1: answered (R7).\n- Q2: answered (R5a).\n";
    assert_eq!(parse(md), Vec::<String>::new());
}

#[test]
fn real_question_before_none_stays() {
    assert_eq!(parse("## Open questions\n- Which db?\n- None.\n").len(), 2);
}

#[test]
fn none_of_the_options_is_a_question() {
    assert_eq!(
        parse("## Open questions\n- None of these fit, which one?\n"),
        ["None of these fit, which one?"]
    );
}
