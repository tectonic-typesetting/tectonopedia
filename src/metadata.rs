// Copyright 2022 the Tectonic Project
// Licensed under the MIT License

//! Metadata entries output to the `pedia.txt` file by the TeX passes.

use tectonic_errors::prelude::*;

use crate::index::{IndexRefFlag, IndexRefFlags};

/// A metadata entry from the `pedia.txt` file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metadatum<'a> {
    /// Declare an output HTML file that is created by this input. The value is
    /// the relative path of the output HTML file. without escaping.
    Output(&'a str),

    /// Define the location of an index entry.
    IndexDef {
        /// The name of the index for which this entry is being declared.
        index: &'a str,

        /// The name of the entry being declared.
        entry: &'a str,

        /// The URL fragment specifying the location within the current output
        /// document that is best associated with this entry's definition. May
        /// be empty. For HTML, should otherwise have the form `"#frag"`.
        fragment: &'a str,
    },

    /// Reference an index entry.
    ///
    /// For the second processing pass, the reference will be resolved and the
    /// resolved value will be provided to the TeX code.
    IndexRef {
        /// The name of the index in which the entry is being referenced.
        index: &'a str,

        /// The name of the entry being referenced.
        entry: &'a str,

        /// The kinds of resources required by this reference.
        flags: IndexRefFlags,
    },

    /// Define the primary textual representation associated with an index entry.
    IndexText {
        /// The name of the index for which this entry is being declared.
        index: &'a str,

        /// The name of the entry being declared.
        entry: &'a str,

        /// The full TeX representation of the entry.
        tex: &'a str,

        /// The plain-text representation of the entry.
        plain: &'a str,
    },
}

impl<'a> Metadatum<'a> {
    pub fn parse(s: &'a str) -> Result<Self> {
        // It seems that we can't use FromStr because we can't link up the
        // lifetime in the input argument here to the impl lifetime.
        let (cseq, terms) = parse_cseq_line(s)?;
        let terms: Result<Vec<_>> = terms.collect();
        let terms = terms?;

        match cseq {
            "output" => {
                ensure!(terms.len() == 1, "malformed metadata line {:?}: \\output must be followed by exactly 1 braced term", s);
                Ok(Metadatum::Output(terms[0]))
            }

            "idef" => {
                ensure!(terms.len() == 3, "malformed metadata line {:?}: \\idef must be followed by exactly 3 braced terms", s);
                Ok(Metadatum::IndexDef {
                    index: terms[0],
                    entry: terms[1],
                    fragment: terms[2],
                })
            }

            "iref" => {
                ensure!(terms.len() == 3, "malformed metadata line {:?}: \\iref must be followed by exactly 3 braced terms", s);

                let index = terms[0];
                let entry = terms[1];
                let flags_term = terms[2];
                let mut flags = 0;

                if flags_term.contains('l') {
                    flags |= IndexRefFlag::NeedsLoc as u8;
                }

                if flags_term.contains('t') {
                    flags |= IndexRefFlag::NeedsText as u8;
                }

                Ok(Metadatum::IndexRef {
                    index,
                    entry,
                    flags,
                })
            }

            "itext" => {
                ensure!(terms.len() == 4, "malformed metadata line {:?}: \\itext must be followed by exactly 4 braced terms", s);
                Ok(Metadatum::IndexText {
                    index: terms[0],
                    entry: terms[1],
                    tex: terms[2],
                    plain: terms[3],
                })
            }

            _ => {
                bail!("unrecognized metadata line {:?}", s)
            }
        }
    }
}

/// Parse a string of the form `\CSEQ{A}{B}{C}` into the control sequence and an
/// interator of the individual terms.
fn parse_cseq_line(s: &str) -> Result<(&str, CseqLineTerms<'_>)> {
    let mut it = s.char_indices();

    match it.next() {
        Some((0, '\\')) => {}
        _ => bail!("cseq-line {:?} did not start with `\\`", s),
    };

    let cseq_end = loop {
        match it.next() {
            Some((i, '{')) => break i,
            Some(_) => {}
            None => {
                // It's OK if there aren't any "terms"
                return Ok((
                    &s[1..],
                    CseqLineTerms {
                        s,
                        it,
                        first_i0: None,
                    },
                ));
            }
        }
    };

    Ok((
        &s[1..cseq_end],
        CseqLineTerms {
            s,
            it,
            first_i0: Some(cseq_end + 1),
        },
    ))
}

/// Helper type for parsing a line of the form `\cseq{t1}{t2}{t3}` into the
/// sequence of "terms" `t1`, `t2`, and `t3`. Nested braces are honored.
/// Unexpected text before, after, or between the braced terms is an error.
struct CseqLineTerms<'a> {
    s: &'a str,
    it: std::str::CharIndices<'a>,
    first_i0: Option<usize>,
}

impl<'a> Iterator for CseqLineTerms<'a> {
    type Item = Result<&'a str>;

    fn next(&mut self) -> Option<Result<&'a str>> {
        // In steady-state at this point, the next character should either be a
        // `{`, or we're finished. But since we can only detect the end of the
        // "cseq" portion by reading the `{`, for the first term we start in the
        // middle of things, without a leading brace to read.

        let i0 = if let Some(fi0) = self.first_i0.take() {
            fi0
        } else {
            match self.it.next() {
                Some((i, '{')) => i + 1,

                Some((i, _)) => {
                    return Some(Err(anyhow!(
                        "unexpected character between terms at index {} in cseq-line {:?}",
                        i,
                        self.s
                    )))
                }

                None => return None,
            }
        };

        // Now we scan until we close out this term.

        let mut depth = 0;

        let i1 = loop {
            match self.it.next() {
                Some((_, '{')) => {
                    depth += 1;
                }

                Some((i, '}')) => {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        break i;
                    }
                }

                Some(_) => {}

                None => {
                    return Some(Err(anyhow!(
                        "incomplete/unbalanced terms in cseq-line {:?}",
                        self.s
                    )))
                }
            }
        };

        Some(Ok(&self.s[i0..i1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cseq_line_1() {
        assert!(parse_cseq_line("noslash").is_err());

        fn parse_collect(s: &str) -> Result<(&str, Vec<&str>)> {
            let (cs, terms) = parse_cseq_line(s)?;
            let terms: Result<Vec<_>> = terms.collect();
            let terms = terms?;
            Ok((cs, terms))
        }

        let (cs, terms) = parse_collect("\\noterms").unwrap();
        assert!(cs == "noterms");
        assert!(terms.is_empty());

        let (cs, terms) = parse_collect("\\t{a}{b}{c}").unwrap();
        assert!(cs == "t");
        assert!(terms.len() == 3);
        assert!(terms[0] == "a");
        assert!(terms[1] == "b");
        assert!(terms[2] == "c");

        let (cs, terms) = parse_collect("\\t {a{b}c}").unwrap();
        assert!(cs == "t ");
        assert!(terms.len() == 1);
        assert!(terms[0] == "a{b}c");

        assert!(parse_collect("\\t{a").is_err());
        assert!(parse_collect("\\t{a{}").is_err());
        assert!(parse_collect("\\t{a}}").is_err());
        assert!(parse_collect("\\t{a} {b}").is_err());
        assert!(parse_collect("\\t{a}x{b}").is_err());
        assert!(parse_collect("\\t{a}{b}x").is_err());
    }
}
