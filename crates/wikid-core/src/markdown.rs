//! Shared Markdown scanning helpers.

/// Tracks CommonMark-style fenced code blocks while scanning a document line by line.
#[derive(Debug, Default, Clone)]
pub(crate) struct FenceTracker {
	open: Option<FenceMarker>,
}

impl FenceTracker {
	pub(crate) fn new() -> Self {
		Self::default()
	}

	/// Observes one line and updates fence state.
	///
	/// Returns `true` when the line is an opening or matching closing fence line.
	/// Fence lines themselves should usually be skipped by structural scanners.
	pub(crate) fn observe(&mut self, line: &str) -> bool {
		let Some(marker) = fence_marker(line.trim_start()) else {
			return false;
		};
		match self.open {
			Some(open) if marker.closes(open) => self.open = None,
			None => self.open = Some(marker),
			_ => return false,
		}
		true
	}

	pub(crate) fn in_fence(&self) -> bool {
		self.open.is_some()
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceMarker {
	delimiter: u8,
	len: usize,
}

impl FenceMarker {
	fn closes(self, open: Self) -> bool {
		self.delimiter == open.delimiter && self.len >= open.len
	}
}

fn fence_marker(trimmed_line: &str) -> Option<FenceMarker> {
	let delimiter = match trimmed_line.as_bytes().first() {
		Some(b'`') => b'`',
		Some(b'~') => b'~',
		_ => return None,
	};
	let len = trimmed_line.bytes().take_while(|b| *b == delimiter).count();
	(len >= 3).then_some(FenceMarker { delimiter, len })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tracks_matching_delimiter_and_minimum_closing_length() {
		let mut tracker = FenceTracker::new();
		assert!(tracker.observe("````rust"));
		assert!(tracker.in_fence());
		assert!(!tracker.observe("~~~"));
		assert!(tracker.in_fence());
		assert!(!tracker.observe("```"));
		assert!(tracker.in_fence());
		assert!(tracker.observe("````"));
		assert!(!tracker.in_fence());
	}
}
