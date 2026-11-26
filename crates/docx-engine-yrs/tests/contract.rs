//! Bounded checks of the paired-range storage contract in spec R4.
mod pairs;
use pairs::Peer;

/// What a reader finds for one character or paragraph mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// The character, or none for a paragraph mark.
    pub c: Option<char>,
    pub basis: Option<String>,
    pub revision: Option<String>,
    /// Bold as it is set, or none where it is not set.
    pub bold: Option<String>,
    pub comments: Vec<String>,
}

/// Two replicas that both hold `text` with basis `R1`.
fn both(text: &str) -> (Peer, Peer) {
    let (mut alice, mut bob) = (Peer::new(1), Peer::new(2));
    alice.insert(0, text, "R1", None, None);
    Peer::sync(&mut alice, &mut bob, true);
    (alice, bob)
}

/// Syncs, requires both to display the same, and returns it.
fn display(alice: &mut Peer, bob: &mut Peer) -> Vec<Unit> {
    Peer::sync(alice, bob, true);
    let shown = alice.read();
    assert_eq!(shown, bob.read());
    shown
}

/// The text shown, with `¶` for a paragraph mark.
fn text(shown: &[Unit]) -> String {
    shown.iter().map(|unit| unit.c.unwrap_or('¶')).collect()
}

fn of(shown: &[Unit], c: char) -> &Unit {
    let mut found = shown.iter().filter(|unit| unit.c == Some(c));
    let one = found.next().unwrap_or_else(|| panic!("{c} is shown"));
    assert!(found.next().is_none(), "{c} is shown once");
    one
}

fn bold(shown: &[Unit], c: char) -> Option<&str> {
    of(shown, c).bold.as_deref()
}

fn basis(shown: &[Unit], c: char) -> &str {
    of(shown, c).basis.as_deref().unwrap()
}

#[test]
fn row_1_deletions_on_each_side_of_plain_text_leave_bold_on_the_rest() {
    let (mut alice, mut bob) = both("paragraph");
    alice.bold(0, 2, Some("on"));
    alice.bold(3, 6, Some("on"));
    Peer::sync(&mut alice, &mut bob, true);
    alice.delete(0, 2);
    bob.delete(2, 1);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "agraph");
    assert!(shown.iter().all(|unit| unit.bold.as_deref() == Some("on")));
}

#[test]
fn row_2_text_typed_into_text_another_deletes_keeps_what_it_stated() {
    let (mut alice, mut bob) = both("abcde");
    alice.insert(4, "Z", "R1", None, None);
    bob.delete(0, 5);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "Z");
    assert_eq!(basis(&shown, 'Z'), "R1");
}

#[test]
fn row_3_deletions_on_each_side_of_text_leave_each_basis() {
    let (mut alice, mut bob) = both("AB");
    alice.insert(2, "CD", "R2", None, None);
    alice.insert(3, "Q", "R1", None, None);
    Peer::sync(&mut alice, &mut bob, true);
    alice.delete(1, 2);
    bob.delete(3, 1);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "AD");
    assert_eq!((basis(&shown, 'A'), basis(&shown, 'D')), ("R1", "R2"));
}

#[test]
fn row_4_bold_over_two_ranges_that_overlap_is_over_both() {
    let (mut alice, mut bob) = both("abcde");
    alice.bold(1, 2, Some("on"));
    bob.bold(2, 3, Some("on"));
    let shown = display(&mut alice, &mut bob);
    assert_eq!(bold(&shown, 'a'), None);
    for c in "bcde".chars() {
        assert_eq!(bold(&shown, c), Some("on"), "{c}");
    }
}

#[test]
fn row_5_bold_set_on_and_off_at_once_leaves_one_of_them() {
    let (mut alice, mut bob) = both("abcde");
    alice.bold(1, 3, Some("on"));
    bob.bold(2, 2, Some("off"));
    let shown = display(&mut alice, &mut bob);
    assert_eq!(bold(&shown, 'b'), Some("on"));
    assert!(bold(&shown, 'c').is_some());
    assert_eq!(bold(&shown, 'c'), bold(&shown, 'd'));
}

#[test]
fn row_6_text_typed_inside_a_range_another_bolds_is_bold() {
    let (mut alice, mut bob) = both("abcde");
    alice.bold(1, 3, Some("on"));
    bob.insert(3, "Q", "R2", None, None);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "abcQde");
    assert_eq!(bold(&shown, 'Q'), Some("on"));
    assert_eq!(basis(&shown, 'Q'), "R2");
}

#[test]
fn row_7_text_typed_at_the_edge_of_a_range_is_the_same_for_both() {
    // The contract permits either, so both are accepted. Which it is can
    // turn on who has the higher name, so both ways round are run.
    for bolder in [1, 2] {
        let (mut alice, mut bob) = (Peer::new(bolder), Peer::new(3 - bolder));
        alice.insert(0, "abc", "R1", None, None);
        Peer::sync(&mut alice, &mut bob, true);
        alice.bold(1, 2, Some("on"));
        bob.insert(3, "Q", "R1", None, None);
        let shown = display(&mut alice, &mut bob);
        assert_eq!(text(&shown), "abcQ");
        assert!(matches!(bold(&shown, 'Q'), None | Some("on")));
        assert_eq!(bold(&shown, 'c'), Some("on"));
    }
}

#[test]
fn row_8_text_typed_inside_a_tracked_insertion_states_its_own_revision() {
    let (mut alice, mut bob) = (Peer::new(1), Peer::new(2));
    alice.insert(0, "wx", "R1", Some("alice"), None);
    Peer::sync(&mut alice, &mut bob, true);
    bob.insert(1, "Q", "R1", Some("bob"), None);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "wQx");
    let revision = |c| of(&shown, c).revision.as_deref();
    assert_eq!(
        (revision('w'), revision('x')),
        (Some("alice"), Some("alice"))
    );
    assert_eq!(revision('Q'), Some("bob"));
}

#[test]
fn row_9_two_comments_that_overlap_are_both_over_the_overlap() {
    let (mut alice, mut bob) = both("abcde");
    alice.comment(1, 2, "alice's");
    bob.comment(2, 3, "bob's");
    let shown = display(&mut alice, &mut bob);
    let comments = |c| of(&shown, c).comments.join(", ");
    assert_eq!(comments('a'), "");
    assert_eq!(comments('b'), "alice's");
    assert_eq!(comments('c'), "alice's, bob's");
    assert_eq!(comments('d'), "bob's");
    assert_eq!(comments('e'), "bob's");
}

#[test]
fn row_10_two_marks_at_one_place_give_an_empty_paragraph() {
    let (mut alice, mut bob) = both("ab");
    alice.enter(1);
    bob.enter(1);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "a¶¶b");
}

#[test]
fn row_11_an_undone_deletion_comes_back_around_what_was_typed_in_it() {
    let (mut alice, mut bob) = both("abcde");
    alice.delete(1, 3);
    bob.insert(3, "Z", "R2", None, None);
    Peer::sync(&mut alice, &mut bob, true);
    alice.undo();
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "abcZde");
    for c in "abcde".chars() {
        assert_eq!(basis(&shown, c), "R1", "{c}");
    }
    assert_eq!(basis(&shown, 'Z'), "R2");
}

#[test]
fn row_12_a_replica_that_is_opened_again_still_keeps_each_basis() {
    let (alice, mut bob) = both("AB");
    let mut alice = alice.reopen();
    alice.insert(1, "Q", "R2", None, None);
    let shown = display(&mut alice, &mut bob);
    assert_eq!(text(&shown), "AQB");
    assert_eq!(
        (basis(&shown, 'A'), basis(&shown, 'Q'), basis(&shown, 'B')),
        ("R1", "R2", "R1")
    );
}

/// One edit of the set the runs below take theirs from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edit {
    /// One character before this position.
    Insert(usize),
    Delete(usize, usize),
    Bold(usize, usize, &'static str),
    Comment(usize, usize),
    Enter(usize),
}

const TEXT: &str = "abcde";
/// Bold is on over `bc` before the edits, and `de` is a tracked insertion.
const BOLD: [Option<&str>; 5] = [None, Some("on"), Some("on"), None, None];
const REVISION: [Option<&str>; 5] = [None, None, None, Some("first"), Some("first")];

/// Every edit: a character or a mark at each of the six places, a deletion
/// of one or two characters, bold on or off over one to three, and a
/// comment over one to three.
fn edits() -> Vec<Edit> {
    let mut edits = Vec::new();
    for at in 0..=TEXT.len() {
        edits.extend([Edit::Insert(at), Edit::Enter(at)]);
        for len in 1..=3 {
            if at + len > TEXT.len() {
                continue;
            }
            edits.extend([Edit::Bold(at, len, "on"), Edit::Bold(at, len, "off")]);
            edits.push(Edit::Comment(at, len));
            if len <= 2 {
                edits.push(Edit::Delete(at, len));
            }
        }
    }
    edits
}

/// One person's part in a run: the edit, the character an insert puts, and
/// what that character states. A comment is named after the character.
#[derive(Clone, Copy)]
struct Part {
    edit: Edit,
    c: char,
    basis: &'static str,
    revision: Option<&'static str>,
}

const PARTS: [Part; 3] = [
    Part {
        edit: Edit::Enter(0),
        c: 'X',
        basis: "RA",
        revision: None,
    },
    Part {
        edit: Edit::Enter(0),
        c: 'Y',
        basis: "RB",
        revision: Some("second"),
    },
    Part {
        edit: Edit::Enter(0),
        c: 'Z',
        basis: "RC",
        revision: Some("third"),
    },
];

impl Part {
    fn apply(&self, way: &mut Peer) {
        match self.edit {
            Edit::Insert(at) => {
                let c = self.c.to_string();
                way.insert(at, &c, self.basis, self.revision, self.stated());
            }
            Edit::Delete(at, len) => way.delete(at, len),
            Edit::Bold(at, len, value) => way.bold(at, len, Some(value)),
            Edit::Comment(at, len) => way.comment(at, len, &self.c.to_string()),
            Edit::Enter(at) => way.enter(at),
        }
    }

    /// Typed text states the bold of the text before it.
    fn stated(&self) -> Option<&'static str> {
        match self.edit {
            Edit::Insert(at) => at.checked_sub(1).and_then(|before| BOLD[before]),
            _ => None,
        }
    }

    /// The range this part puts, if it puts one.
    fn range(&self) -> Option<(usize, usize)> {
        match self.edit {
            Edit::Bold(at, len, _) | Edit::Comment(at, len) => Some((at, at + len)),
            _ => None,
        }
    }

    fn bolds(&self, i: usize) -> Option<&'static str> {
        match self.edit {
            Edit::Bold(at, len, value) if (at..at + len).contains(&i) => Some(value),
            _ => None,
        }
    }

    fn comments(&self, i: usize) -> bool {
        matches!(self.edit, Edit::Comment(at, len) if (at..at + len).contains(&i))
    }
}

/// Whether the marks shown can each be one of the marks put: after the text
/// that was before it, and before the rest.
fn marks_fit(marks: &[usize], wanted: &[usize], fits: &dyn Fn(usize, usize) -> bool) -> bool {
    match (marks, wanted) {
        ([], []) => true,
        ([mark, rest @ ..], _) if marks.len() == wanted.len() => (0..wanted.len()).any(|i| {
            let mut others = wanted.to_vec();
            fits(*mark, others.remove(i)) && marks_fit(rest, &others, fits)
        }),
        _ => false,
    }
}

/// What the contract requires and permits after the edits in `parts` were
/// made at the same time and synced. `restored` is a deletion that was made
/// and then undone. Returns what is wrong, if anything.
fn check(shown: &[Unit], parts: &[Part], restored: Option<Edit>) -> Result<(), String> {
    let place = |c: char| shown.iter().position(|unit| unit.c == Some(c));
    // Content that two people both deleted comes back when one undoes.
    let back =
        |i: usize| matches!(restored, Some(Edit::Delete(at, len)) if (at..at + len).contains(&i));
    let gone = |i: usize| {
        let deleted =
            |part: &Part| matches!(part.edit, Edit::Delete(at, len) if (at..at + len).contains(&i));
        parts.iter().any(deleted) && !back(i)
    };
    // Exact accounting also rejects a failed undo, an extra insertion and
    // a repeated insertion. The base characters keep their relative order.
    let expected: String = TEXT
        .chars()
        .enumerate()
        .filter(|(i, _)| !gone(*i))
        .map(|(_, c)| c)
        .collect();
    let base: String = shown
        .iter()
        .filter_map(|unit| unit.c)
        .filter(|c| TEXT.contains(*c))
        .collect();
    if base != expected {
        return Err(format!("base text {base:?}, expected {expected:?}"));
    }
    for c in shown.iter().filter_map(|unit| unit.c) {
        if !TEXT.contains(c)
            && !parts
                .iter()
                .any(|part| matches!(part.edit, Edit::Insert(_)) && part.c == c)
        {
            return Err(format!("unexpected character {c}"));
        }
    }
    for part in parts
        .iter()
        .filter(|part| matches!(part.edit, Edit::Insert(_)))
    {
        let count = shown.iter().filter(|unit| unit.c == Some(part.c)).count();
        if count != 1 {
            return Err(format!("{} is shown {count} times", part.c));
        }
    }
    // The ranges put at once over each character, and the bold it shows.
    let mut held: Vec<(Vec<&str>, Option<&str>)> = Vec::new();
    for (i, c) in TEXT.chars().enumerate() {
        let found: Vec<&Unit> = shown.iter().filter(|unit| unit.c == Some(c)).collect();
        if gone(i) != found.is_empty() || found.len() > 1 {
            return Err(format!("{c} is shown {} times", found.len()));
        }
        let Some(unit) = found.first() else {
            continue;
        };
        if unit.basis.as_deref() != Some("R1") || unit.revision.as_deref() != REVISION[i] {
            return Err(format!("{c} lost what it stated: {unit:?}"));
        }
        let over: Vec<&str> = parts.iter().filter_map(|part| part.bolds(i)).collect();
        let right = match over[..] {
            [] => unit.bold.as_deref() == BOLD[i],
            [value] => unit.bold.as_deref() == Some(value),
            _ => over.contains(&unit.bold.as_deref().unwrap_or("")),
        };
        if !right {
            return Err(format!(
                "{c} has bold {:?}, with {over:?} put over it",
                unit.bold
            ));
        }
        let by: Vec<&str> = (parts.iter().filter(|part| part.bolds(i).is_some()))
            .map(|part| part.basis)
            .collect();
        held.push((by, unit.bold.as_deref()));
        let named: Vec<String> = (parts.iter().filter(|part| part.comments(i)))
            .map(|part| part.c.to_string())
            .collect();
        if unit.comments != named {
            return Err(format!(
                "{c} has comments {:?}, not {named:?}",
                unit.comments
            ));
        }
    }
    // Where the same ranges were put at once, the same one of them holds.
    for (by, bold) in &held {
        if by.len() > 1
            && held
                .iter()
                .any(|(other, shown)| other == by && shown != bold)
        {
            return Err(format!("ranges put at once hold in parts: {held:?}"));
        }
    }
    for part in parts {
        let Edit::Insert(at) = part.edit else {
            continue;
        };
        let Some(here) = place(part.c) else {
            return Err(format!("{} is not shown", part.c));
        };
        let unit = &shown[here];
        if unit.basis.as_deref() != Some(part.basis) || unit.revision.as_deref() != part.revision {
            return Err(format!("{} lost what it stated: {unit:?}", part.c));
        }
        // It is after the text that was before it, and before the rest.
        for (i, c) in TEXT.chars().enumerate() {
            if place(c).is_some_and(|other| (i < at) != (other < here)) {
                return Err(format!("{} is on the wrong side of {c}", part.c));
            }
        }
        // A range put at the same time over both its neighbours is over it
        // too. One that ends or starts beside it may be. No other is.
        let others = parts.iter().filter(|other| other.c != part.c);
        let around = |inside: bool| {
            let reaches = move |other: &&Part| {
                let Some((from, to)) = other.range() else {
                    return false;
                };
                match inside {
                    true => from < at && at < to,
                    false => at == from || at == to,
                }
            };
            others.clone().filter(reaches)
        };
        let bold = |other: &Part| match other.edit {
            Edit::Bold(_, _, value) => Some(Some(value)),
            _ => None,
        };
        let mut permitted: Vec<Option<&str>> = around(true).filter_map(bold).collect();
        if permitted.is_empty() {
            permitted.push(part.stated());
        }
        permitted.extend(around(false).filter_map(bold));
        if !permitted.contains(&unit.bold.as_deref()) {
            return Err(format!(
                "{} has bold {:?}, not one of {permitted:?}",
                part.c, unit.bold
            ));
        }
        let comment =
            |other: &Part| matches!(other.edit, Edit::Comment(..)).then(|| other.c.to_string());
        let must: Vec<String> = around(true).filter_map(comment).collect();
        let may: Vec<String> = around(false).filter_map(comment).collect();
        let all = must.iter().all(|name| unit.comments.contains(name));
        let only = (unit.comments.iter()).all(|name| must.contains(name) || may.contains(name));
        if !all || !only {
            return Err(format!("{} has comments {:?}", part.c, unit.comments));
        }
    }
    let wanted: Vec<usize> = (parts.iter())
        .filter_map(|part| match part.edit {
            Edit::Enter(at) => Some(at),
            _ => None,
        })
        .collect();
    let marks: Vec<usize> = (0..shown.len()).filter(|i| shown[*i].c.is_none()).collect();
    let fits = |mark: usize, at: usize| {
        let mut text = TEXT.chars().enumerate();
        text.all(|(i, c)| place(c).is_none_or(|other| (i < at) == (other < mark)))
    };
    if !marks_fit(&marks, &wanted, &fits) {
        return Err(format!("marks at {marks:?} for Enter at {wanted:?}"));
    }
    Ok(())
}

/// Replicas that all hold `abcde` with one basis, bold on over `bc`, and
/// `de` a tracked insertion.
fn seeded(count: u8) -> Vec<Peer> {
    let mut ways: Vec<Peer> = (1..=count).map(Peer::new).collect();
    let (first, rest) = ways.split_first_mut().unwrap();
    first.insert(0, "abc", "R1", None, None);
    first.insert(3, "de", "R1", Some("first"), None);
    first.bold(1, 2, Some("on"));
    rest.iter_mut().for_each(|way| Peer::sync(first, way, true));
    ways
}

/// Every pair of edits, made at the same time on one document and synced.
///
/// The bounds: the document is `abcde`, and an edit is one of the 57 that
/// `edits` lists, so 3,249 pairs. Each pair runs five times: in both orders
/// of delivery, in both again with the first person undoing after the sync,
/// and once with the first person closing and opening before the edit.
/// That is 16,245 runs for each way.
///
/// Nothing is compared with another way's result. A run passes when both
/// replicas display the same and `check` finds what the contract requires
/// or permits.
#[test]
fn every_pair_of_edits() {
    let mut wrong = Vec::new();
    let edits = edits();
    assert_eq!(edits.len(), 57);
    let runs = [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (false, true, false),
        (true, false, true),
    ];
    for (first, second) in edits
        .iter()
        .flat_map(|a| edits.iter().map(move |b| (*a, *b)))
    {
        for (a_first, undone, reopened) in runs {
            let mut ways = seeded(2).into_iter();
            let (mut alice, mut bob) = (ways.next().unwrap(), ways.next().unwrap());
            if reopened {
                alice = alice.reopen();
            }
            let (first, second) = (
                Part {
                    edit: first,
                    ..PARTS[0]
                },
                Part {
                    edit: second,
                    ..PARTS[1]
                },
            );
            first.apply(&mut alice);
            second.apply(&mut bob);
            Peer::sync(&mut alice, &mut bob, a_first);
            if undone {
                alice.undo();
                Peer::sync(&mut alice, &mut bob, a_first);
            }
            let shown = alice.read();
            let found = match shown == bob.read() {
                false => Err("the two display different documents".to_string()),
                true if undone => check(&shown, &[second], Some(first.edit)),
                true => check(&shown, &[first, second], None),
            };
            if let Err(what) = found {
                let (first, second) = (first.edit, second.edit);
                wrong.push(format!(
                    "{first:?} and {second:?}, undone {undone}, reopened {reopened}: {what}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} runs wrong, the first: {}",
        wrong.len(),
        wrong[0]
    );
}

/// Every three edits, one each by three people at the same time.
///
/// The bounds: the same document, and of the 57 edits those that delete
/// one character or put a range over one or over three, so 41 edits and
/// 68,921 triples. Each runs once, syncing the first two people, then the
/// last two, then the first two again. No one undoes or reopens.
#[test]
fn every_three_edits() {
    let mut wrong = Vec::new();
    let short = |edit: &Edit| match edit {
        Edit::Delete(_, len) | Edit::Bold(_, len, _) | Edit::Comment(_, len) => *len != 2,
        _ => true,
    };
    let edits: Vec<Edit> = edits().into_iter().filter(short).collect();
    assert_eq!(edits.len(), 41);
    for first in &edits {
        for second in &edits {
            for third in &edits {
                let mut ways = seeded(3);
                let made = [*first, *second, *third];
                let parts: Vec<Part> = (made.iter().zip(PARTS))
                    .map(|(edit, part)| Part {
                        edit: *edit,
                        ..part
                    })
                    .collect();
                (parts.iter().zip(&mut ways)).for_each(|(part, way)| part.apply(way));
                for (one, other) in [(0, 1), (1, 2), (0, 1)] {
                    let (left, right) = ways.split_at_mut(other);
                    Peer::sync(&mut left[one], &mut right[0], true);
                }
                let shown = ways[0].read();
                let found = match ways.iter().all(|way| way.read() == shown) {
                    false => Err("the three display different documents".to_string()),
                    true => check(&shown, &parts, None),
                };
                if let Err(what) = found {
                    wrong.push(format!("{made:?}: {what}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} runs wrong, the first: {}",
        wrong.len(),
        wrong[0]
    );
}

#[test]
fn checker_rejects_reordering_duplicates_and_failed_insertion_undo() {
    let mut ways = seeded(1);
    let original = ways[0].read();
    assert!(check(&original, &[], None).is_ok());
    let mut reversed = original.clone();
    reversed.reverse();
    assert!(check(&reversed, &[], None).is_err());
    let part = Part {
        edit: Edit::Insert(2),
        ..PARTS[0]
    };
    part.apply(&mut ways[0]);
    let inserted = ways[0].read();
    assert!(check(&inserted, &[part], None).is_ok());
    assert!(check(&inserted, &[], Some(part.edit)).is_err());
    let mut duplicated = inserted.clone();
    duplicated.insert(2, inserted[2].clone());
    assert!(check(&duplicated, &[part], None).is_err());
}
