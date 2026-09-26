//! Decoding helpers shared by the hand-written message codecs and the
//! derived payload codecs.
//!
//! Like minicbor's derived decoders, these accept definite and indefinite
//! lengths; the encoders only ever write definite ones.

use minicbor::Decode;
use minicbor::data::Type;
use minicbor::decode::{Decoder, Error};

/// Calls `entry` with every key of the map at the decoder's position;
/// `entry` must consume the value that follows the key.
pub(crate) fn map_entries<'b>(
    d: &mut Decoder<'b>,
    mut entry: impl FnMut(&mut Decoder<'b>, i64) -> Result<(), Error>,
) -> Result<(), Error> {
    match d.map()? {
        Some(n) => {
            for _ in 0..n {
                let key = d.i64()?;
                entry(d, key)?;
            }
        }
        None => {
            while d.datatype()? != Type::Break {
                let key = d.i64()?;
                entry(d, key)?;
            }
            d.skip()?;
        }
    }
    Ok(())
}

/// Skips a map, whatever it holds; fails if the item is not a map.
pub(crate) fn skip_map(d: &mut Decoder<'_>) -> Result<(), Error> {
    map_entries(d, |d, _| d.skip())
}

/// A cursor over the elements of the array (or the entries of the map) at the
/// decoder's position.
pub(crate) struct Elements {
    /// Elements left in a definite array, or `None` inside an indefinite one.
    remaining: Option<u64>,
}

impl Elements {
    pub(crate) fn begin(d: &mut Decoder<'_>) -> Result<Elements, Error> {
        Ok(Elements {
            remaining: d.array()?,
        })
    }

    /// A cursor over the entries of the map at the decoder's position; each
    /// step is one key and its value.
    pub(crate) fn begin_map(d: &mut Decoder<'_>) -> Result<Elements, Error> {
        Ok(Elements {
            remaining: d.map()?,
        })
    }

    /// The definite length, if the array has one.
    pub(crate) fn len(&self) -> Option<u64> {
        self.remaining
    }

    /// Whether another element follows. Consumes the break that ends an
    /// indefinite array.
    pub(crate) fn next(&mut self, d: &mut Decoder<'_>) -> Result<bool, Error> {
        match &mut self.remaining {
            Some(0) => Ok(false),
            Some(n) => {
                *n -= 1;
                Ok(true)
            }
            None => {
                if d.datatype()? == Type::Break {
                    d.skip()?;
                    self.remaining = Some(0);
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Reads element `index` with `read`, failing if the array has ended.
    pub(crate) fn require<'b, T>(
        &mut self,
        d: &mut Decoder<'b>,
        index: i64,
        name: &'static str,
        read: impl FnOnce(&mut Decoder<'b>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if !self.next(d)? {
            return Err(missing(index, name).at(d.position()));
        }
        read(d)
    }

    /// Skips the elements this version does not know.
    pub(crate) fn skip_rest(&mut self, d: &mut Decoder<'_>) -> Result<(), Error> {
        while self.next(d)? {
            d.skip()?;
        }
        Ok(())
    }
}

/// Decodes an array into a `Vec`, reserving the header's length up front.
/// Every element takes at least one byte, so the reservation never exceeds
/// the bytes left in the input, whatever the header claims.
pub(crate) fn decode_vec<'b, Ctx, T: Decode<'b, Ctx>>(
    d: &mut Decoder<'b>,
    ctx: &mut Ctx,
) -> Result<Vec<T>, Error> {
    let mut items = Elements::begin(d)?;
    let mut v = Vec::with_capacity(reserve(items.len(), d));
    while items.next(d)? {
        v.push(T::decode(d, ctx)?);
    }
    Ok(v)
}

/// How many elements to reserve for an array of `len` elements: all of them,
/// but never more than the bytes left in the input.
pub(crate) fn reserve(len: Option<u64>, d: &Decoder<'_>) -> usize {
    let left = d.input().len() - d.position();
    len.map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX).min(left))
}

/// The error for a required field or element that is absent.
pub(crate) fn missing(index: i64, name: &'static str) -> Error {
    Error::missing_value(index).with_message(name)
}

/// Checks the kind read from key or element 0 of a message.
pub(crate) fn expect_kind(found: Option<u8>, expected: u8, at: usize) -> Result<(), Error> {
    match found {
        Some(k) if k == expected => Ok(()),
        Some(k) => Err(Error::message(format!("message kind {k}, expected {expected}")).at(at)),
        None => Err(missing(0, "kind").at(at)),
    }
}
