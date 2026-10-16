//! low-level read / write of the primitive fields used by every section.

use crate::CodecError;

// ───────────────────────────── write ─────────────────────────────

/// `usize` count -> `u32` field.
pub(crate) fn put_count(out: &mut Vec<u8>, n: usize) -> Result<(), CodecError> {
    let n = u32::try_from(n).map_err(|_| CodecError::TooManyRows)?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

/// `u16` length + UTF-8 payload.
pub(crate) fn put_name(out: &mut Vec<u8>, s: &str) -> Result<(), CodecError> {
    let len = u16::try_from(s.len()).map_err(|_| CodecError::NameTooLong)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// `u32` length + raw payload.
pub(crate) fn put_blob(out: &mut Vec<u8>, data: &[u8]) -> Result<(), CodecError> {
    let len = u32::try_from(data.len()).map_err(|_| CodecError::DataTooLong)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    Ok(())
}

/// `u128` hash.
pub(crate) fn put_hash(out: &mut Vec<u8>, hash: u128) {
    out.extend_from_slice(&hash.to_le_bytes());
}

// ───────────────────────────── read ──────────────────────────────

/// bounds-checked cursor over the input.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len()
    }

    /// fails with [`CodecError::TrailingBytes`] if anything is left.
    pub(crate) fn finish(&self) -> Result<(), CodecError> {
        match self.buf.is_empty() {
            true => Ok(()),
            false => Err(CodecError::TrailingBytes),
        }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if n > self.buf.len() {
            return Err(CodecError::UnexpectedEof);
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CodecError> {
        self.array().map(u8::from_le_bytes)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CodecError> {
        self.array().map(u32::from_le_bytes)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, CodecError> {
        self.array().map(u64::from_le_bytes)
    }

    pub(crate) fn hash(&mut self) -> Result<u128, CodecError> {
        self.array().map(u128::from_le_bytes)
    }

    /// `u32` count field. Every counted entry takes at least `min_entry`
    /// bytes, so the returned capacity hint is capped by the remaining input.
    pub(crate) fn count(&mut self, min_entry: usize) -> Result<(usize, usize), CodecError> {
        let n = self.u32()? as usize;
        Ok((n, n.min(self.remaining() / min_entry)))
    }

    /// `u16` length + UTF-8 payload.
    pub(crate) fn name(&mut self) -> Result<String, CodecError> {
        let len = u16::from_le_bytes(self.array()?) as usize;
        let raw = self.take(len)?;
        let s = std::str::from_utf8(raw).map_err(|_| CodecError::InvalidUtf8)?;
        Ok(s.to_owned())
    }

    /// `u32` length + raw payload.
    pub(crate) fn blob(&mut self) -> Result<Vec<u8>, CodecError> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }
}
