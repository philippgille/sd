use std::{borrow::Cow, fs, fs::File, io::prelude::*, path::Path};

use crate::{utils, Error, Result};

use regex::bytes::Regex;

#[cfg(test)]
mod tests;
mod validate;

pub use validate::{validate_replace, InvalidReplaceCapture};

pub(crate) struct Replacer {
    regex: Regex,
    replace_with: Vec<u8>,
    is_literal: bool,
    replacements: usize,
}

impl Replacer {
    pub(crate) fn new(
        look_for: String,
        replace_with: String,
        is_literal: bool,
        flags: Option<String>,
        replacements: usize,
    ) -> Result<Self> {
        let (look_for, replace_with) = if is_literal {
            (regex::escape(&look_for), replace_with.into_bytes())
        } else {
            validate_replace(&replace_with)?;

            (
                look_for,
                utils::unescape(&replace_with)
                    .unwrap_or(replace_with)
                    .into_bytes(),
            )
        };

        let mut regex = regex::bytes::RegexBuilder::new(&look_for);
        regex.multi_line(true);

        if let Some(flags) = flags {
            flags.chars().for_each(|c| {
                #[rustfmt::skip]
                match c {
                    'c' => { regex.case_insensitive(false); },
                    'i' => { regex.case_insensitive(true); },
                    'm' => {},
                    'e' => { regex.multi_line(false); },
                    's' => {
                        if !flags.contains('m') {
                            regex.multi_line(false);
                        }
                        regex.dot_matches_new_line(true);
                    },
                    'w' => {
                        regex = regex::bytes::RegexBuilder::new(&format!(
                            "\\b{}\\b",
                            look_for
                        ));
                    },
                    _ => {},
                };
            });
        };

        Ok(Self {
            regex: regex.build()?,
            replace_with,
            is_literal,
            replacements,
        })
    }

    pub(crate) fn has_matches(&self, content: &[u8]) -> bool {
        self.regex.is_match(content)
    }

    pub(crate) fn check_not_empty(mut file: File) -> Result<()> {
        let mut buf: [u8; 1] = Default::default();
        file.read_exact(&mut buf)?;
        Ok(())
    }

    pub(crate) fn replace<'a>(
        &'a self,
        content: &'a [u8],
    ) -> std::borrow::Cow<'a, [u8]> {
        let regex = &self.regex;
        let limit = self.replacements;
        let use_color = false;
        if self.is_literal {
            Self::replacen(
                regex,
                limit,
                content,
                use_color,
                regex::bytes::NoExpand(&self.replace_with),
            )
        } else {
            Self::replacen(
                regex,
                limit,
                content,
                use_color,
                &*self.replace_with,
            )
        }
    }

    /// A modified form of [`regex::bytes::Regex::replacen`] that supports
    /// coloring replacements
    pub(crate) fn replacen<'haystack, R: regex::bytes::Replacer>(
        regex: &regex::bytes::Regex,
        limit: usize,
        haystack: &'haystack [u8],
        use_color: bool,
        mut rep: R,
    ) -> Cow<'haystack, [u8]> {
        let mut it = regex.captures_iter(haystack).enumerate().peekable();
        if it.peek().is_none() {
            return Cow::Borrowed(haystack);
        }
        let mut new = Vec::with_capacity(haystack.len());
        let mut last_match = 0;
        for (i, cap) in it {
            // unwrap on 0 is OK because captures only reports matches
            let m = cap.get(0).unwrap();
            new.extend_from_slice(&haystack[last_match..m.start()]);
            if use_color {
                new.extend_from_slice(
                    ansi_term::Color::Blue.prefix().to_string().as_bytes(),
                );
            }
            rep.replace_append(&cap, &mut new);
            if use_color {
                new.extend_from_slice(
                    ansi_term::Color::Blue.suffix().to_string().as_bytes(),
                );
            }
            last_match = m.end();
            if limit > 0 && i >= limit - 1 {
                break;
            }
        }
        new.extend_from_slice(&haystack[last_match..]);
        Cow::Owned(new)
    }

    pub(crate) fn replace_preview<'a>(
        &self,
        content: &'a [u8],
    ) -> std::borrow::Cow<'a, [u8]> {
        let regex = &self.regex;
        let limit = self.replacements;
        // TODO: refine this condition more
        let use_color = true;
        if self.is_literal {
            Self::replacen(
                regex,
                limit,
                content,
                use_color,
                regex::bytes::NoExpand(&self.replace_with),
            )
        } else {
            Self::replacen(
                regex,
                limit,
                content,
                use_color,
                &*self.replace_with,
            )
        }
    }

    pub(crate) fn replace_file(&self, path: &Path) -> Result<()> {
        use memmap2::{Mmap, MmapMut};
        use std::ops::DerefMut;

        if Self::check_not_empty(File::open(path)?).is_err() {
            return Ok(());
        }

        let source = File::open(path)?;
        let meta = fs::metadata(path)?;
        let mmap_source = unsafe { Mmap::map(&source)? };
        let replaced = self.replace(&mmap_source);

        let target = tempfile::NamedTempFile::new_in(
            path.parent()
                .ok_or_else(|| Error::InvalidPath(path.to_path_buf()))?,
        )?;
        let file = target.as_file();
        file.set_len(replaced.len() as u64)?;
        file.set_permissions(meta.permissions())?;

        if !replaced.is_empty() {
            let mut mmap_target = unsafe { MmapMut::map_mut(file)? };
            mmap_target.deref_mut().write_all(&replaced)?;
            mmap_target.flush_async()?;
        }

        drop(mmap_source);
        drop(source);

        target.persist(fs::canonicalize(path)?)?;
        Ok(())
    }
}
