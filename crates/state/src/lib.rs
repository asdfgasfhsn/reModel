//! Save states: a small binary serialisation for emulator state.
//!
//! [`State`] saves a value and loads it back **in place**: a state is loaded
//! into a freshly built machine, so whatever a type leaves out (ROM contents,
//! caches, configuration, diagnostics) keeps its constructed value. Structs
//! list the fields that are their state with [`fields!`]; the rest are
//! skipped. Numbers are little-endian; collections carry their length.

use std::collections::VecDeque;

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let b = self.buf.get(self.pos..self.pos + n).ok_or_else(|| Error("state truncated".into()))?;
        self.pos += n;
        Ok(b)
    }

    /// True if everything has been read.
    pub fn done(&self) -> bool {
        self.pos == self.buf.len()
    }
}

pub trait State {
    fn save(&self, w: &mut Writer);
    fn load(&mut self, r: &mut Reader) -> Result<(), Error>;

    /// Save a run of values (byte-sized types override this for speed).
    fn save_all(items: &[Self], w: &mut Writer)
    where
        Self: Sized,
    {
        items.iter().for_each(|i| i.save(w));
    }

    fn load_all(items: &mut [Self], r: &mut Reader) -> Result<(), Error>
    where
        Self: Sized,
    {
        items.iter_mut().try_for_each(|i| i.load(r))
    }
}

macro_rules! number {
    ($($t:ty),*) => {$(
        impl State for $t {
            fn save(&self, w: &mut Writer) {
                w.buf.extend_from_slice(&self.to_le_bytes());
            }
            fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
                *self = <$t>::from_le_bytes(r.bytes(size_of::<$t>())?.try_into().unwrap());
                Ok(())
            }
            fn save_all(items: &[Self], w: &mut Writer) {
                w.buf.reserve(size_of_val(items));
                items.iter().for_each(|i| w.buf.extend_from_slice(&i.to_le_bytes()));
            }
            fn load_all(items: &mut [Self], r: &mut Reader) -> Result<(), Error> {
                let b = r.bytes(size_of_val(items))?;
                for (i, c) in items.iter_mut().zip(b.chunks_exact(size_of::<$t>())) {
                    *i = <$t>::from_le_bytes(c.try_into().unwrap());
                }
                Ok(())
            }
        }
    )*};
}

number!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl State for usize {
    fn save(&self, w: &mut Writer) {
        (*self as u64).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        let mut v = 0u64;
        v.load(r)?;
        *self = v as usize;
        Ok(())
    }
}

impl State for bool {
    fn save(&self, w: &mut Writer) {
        w.buf.push(*self as u8);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        *self = r.bytes(1)?[0] != 0;
        Ok(())
    }
}

impl<T: State, const N: usize> State for [T; N] {
    fn save(&self, w: &mut Writer) {
        T::save_all(self, w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        T::load_all(self, r)
    }
}

fn load_len(r: &mut Reader) -> Result<usize, Error> {
    let mut n = 0u64;
    n.load(r)?;
    usize::try_from(n).map_err(|_| Error("bad length".into()))
}

impl<T: State + Default> State for Vec<T> {
    fn save(&self, w: &mut Writer) {
        (self.len() as u64).save(w);
        T::save_all(self, w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        let n = load_len(r)?;
        self.clear();
        self.resize_with(n, T::default);
        T::load_all(self, r)
    }
}

impl<T: State + Default> State for VecDeque<T> {
    fn save(&self, w: &mut Writer) {
        (self.len() as u64).save(w);
        self.iter().for_each(|i| i.save(w));
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        let n = load_len(r)?;
        self.clear();
        for _ in 0..n {
            let mut v = T::default();
            v.load(r)?;
            self.push_back(v);
        }
        Ok(())
    }
}

impl<T: State + Default> State for Option<T> {
    fn save(&self, w: &mut Writer) {
        self.is_some().save(w);
        if let Some(v) = self {
            v.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        let mut some = false;
        some.load(r)?;
        if some {
            self.get_or_insert_with(T::default).load(r)
        } else {
            *self = None;
            Ok(())
        }
    }
}

impl<T: State + ?Sized> State for Box<T> {
    fn save(&self, w: &mut Writer) {
        (**self).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
        (**self).load(r)
    }
}

macro_rules! tuple {
    ($($n:tt $t:ident),*) => {
        impl<$($t: State),*> State for ($($t,)*) {
            fn save(&self, w: &mut Writer) {
                $(self.$n.save(w);)*
            }
            fn load(&mut self, r: &mut Reader) -> Result<(), Error> {
                $(self.$n.load(r)?;)*
                Ok(())
            }
        }
    };
}

tuple!(0 A, 1 B);
tuple!(0 A, 1 B, 2 C);

/// Implement [`State`] for a struct as the listed fields, in order; the
/// others are left as they are on load.
#[macro_export]
macro_rules! fields {
    ($t:ty { $($f:ident),* $(,)? }) => {
        impl $crate::State for $t {
            fn save(&self, w: &mut $crate::Writer) {
                $( $crate::State::save(&self.$f, w); )*
            }
            fn load(&mut self, r: &mut $crate::Reader) -> Result<(), $crate::Error> {
                $( $crate::State::load(&mut self.$f, r)?; )*
                Ok(())
            }
        }
    };
}

/// Save `value` to bytes.
pub fn to_bytes(value: &impl State) -> Vec<u8> {
    let mut w = Writer::default();
    value.save(&mut w);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Debug, PartialEq)]
    struct S {
        a: u32,
        b: Vec<u8>,
        c: [i16; 3],
        d: Option<(u64, bool)>,
        e: VecDeque<f32>,
        cache: u32,
    }
    fields!(S { a, b, c, d, e });

    #[test]
    fn round_trip_leaves_other_fields() {
        let s = S { a: 7, b: vec![1, 2, 3], c: [-1, 0, 1], d: Some((9, true)), e: [0.5].into(), cache: 1 };
        let bytes = to_bytes(&s);
        let mut t = S { cache: 42, ..S::default() };
        let mut r = Reader::new(&bytes);
        t.load(&mut r).unwrap();
        assert!(r.done());
        assert_eq!(t, S { cache: 42, ..s });
        assert!(S::default().load(&mut Reader::new(&bytes[..5])).is_err());
    }
}
