use gpui::{ClipboardEntry, ClipboardItem, ExternalPaths};
use smallvec::SmallVec;
use std::os::unix::ffi::OsStrExt;
use url::Url;

pub(crate) const FILE_LIST_MIME_TYPE: &str = "text/uri-list";

pub(crate) fn read_file_list(bytes: &[u8]) -> Result<ClipboardItem, &'static str> {
	let text = std::str::from_utf8(bytes).map_err(|_| "file list is not UTF-8")?;
	let mut paths = SmallVec::new();
	for line in text
		.lines()
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
	{
		let url = Url::parse(line).map_err(|_| "file list contains an invalid URI")?;
		if url.query().is_some() || url.fragment().is_some() {
			return Err("file URI contains a query or fragment");
		}
		let path = url
			.to_file_path()
			.map_err(|_| "file list contains a non-local URI")?;
		if path.as_os_str().as_bytes().contains(&0) {
			return Err("file path contains a null byte");
		}
		paths.push(path);
	}
	if paths.is_empty() {
		return Err("file list contains no files");
	}
	Ok(ClipboardItem {
		entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(paths))],
	})
}

/// URI-list clipboard payloads must remain local file entries, not pasted text or
/// partially accepted lists. These tests cover decoding; backend negotiation is
/// exercised separately on an isolated display.
#[cfg(test)]
mod tests {
	use super::*;
	use std::{ffi::OsStr, path::PathBuf};

	#[test]
	fn comments_line_endings_and_escaped_paths_preserve_order() -> Result<(), &'static str> {
		for newline in ["\n", "\r\n"] {
			let input = [
				"# file manager selection",
				"",
				"file:///workspace/a%20b.txt",
				"file://localhost/workspace/%23notes",
				"",
			]
			.join(newline);
			assert_eq!(
				read_file_list(input.as_bytes())?,
				ClipboardItem {
					entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
						smallvec::smallvec![
							PathBuf::from("/workspace/a b.txt"),
							PathBuf::from("/workspace/#notes"),
						]
					))],
				}
			);
		}
		Ok(())
	}

	#[test]
	fn non_utf8_file_names_round_trip_without_loss() -> Result<(), &'static str> {
		let path = PathBuf::from(OsStr::from_bytes(b"/workspace/\xff.txt"));
		let url = Url::from_file_path(&path).map_err(|_| "invalid fixture path")?;
		assert_eq!(
			read_file_list(url.as_str().as_bytes())?,
			ClipboardItem {
				entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
					smallvec::smallvec![path]
				))],
			}
		);
		Ok(())
	}

	#[test]
	fn invalid_lists_never_return_partial_file_selections() {
		for (bytes, error) in [
			(&b""[..], "file list contains no files"),
			(&b"# comment\r\n\r\n"[..], "file list contains no files"),
			(&b"\xff"[..], "file list is not UTF-8"),
			(&b"not a URI"[..], "file list contains an invalid URI"),
			(
				&b"file:///workspace/ok\nhttps://example.com/file"[..],
				"file list contains a non-local URI",
			),
			(
				&b"file://remote/workspace/file"[..],
				"file list contains a non-local URI",
			),
			(
				&b"file:///workspace/file?query"[..],
				"file URI contains a query or fragment",
			),
			(
				&b"file:///workspace/file#fragment"[..],
				"file URI contains a query or fragment",
			),
			(
				&b"file:///workspace/a%00b"[..],
				"file path contains a null byte",
			),
		] {
			assert_eq!(read_file_list(bytes), Err(error), "input: {bytes:?}");
		}
	}
}
