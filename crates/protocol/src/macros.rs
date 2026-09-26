//! Newtypes over the integers the wire carries for enums and bitmasks.

/// Declares a newtype over an unsigned integer that stands for an enum, with
/// a named constant per value this protocol version defines. Other values are
/// kept as they are, so a newer peer's values survive a round trip.
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        pub struct $name:ident($int:ty);
        $(
            $(#[$cmeta:meta])*
            $cname:ident = $value:literal, $label:literal;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub $int);

        impl $name {
            $(
                $(#[$cmeta])*
                pub const $cname: $name = $name($value);
            )*

            /// The snake_case name of a value this protocol version defines.
            pub fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($value => Some($label),)*
                    _ => None,
                }
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => write!(f, "{}({name})", stringify!($name)),
                    None => write!(f, "{}({})", stringify!($name), self.0),
                }
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => f.write_str(name),
                    None => write!(f, "{}", self.0),
                }
            }
        }
    };
}

/// Declares a newtype over a `u32` bitmask with a named constant per bit this
/// protocol version defines. Unknown bits are kept.
macro_rules! wire_flags {
    (
        $(#[$meta:meta])*
        pub struct $name:ident;
        $(
            $(#[$cmeta:meta])*
            $cname:ident = $bit:literal, $label:literal;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub u32);

        impl $name {
            /// No bit set.
            pub const EMPTY: $name = $name(0);
            $(
                $(#[$cmeta])*
                pub const $cname: $name = $name(1 << $bit);
            )*
            const NAMED: &[($name, &'static str)] = &[$(($name(1 << $bit), $label),)*];

            /// Whether every bit of `other` is set in `self`.
            pub const fn contains(self, other: $name) -> bool {
                self.0 & other.0 == other.0
            }

            /// Whether no bit is set.
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// The names of the bits set in `self` that this protocol version
            /// defines, lowest bit first.
            pub fn names(self) -> impl Iterator<Item = &'static str> {
                Self::NAMED
                    .iter()
                    .filter(move |(flag, _)| self.contains(*flag))
                    .map(|(_, name)| *name)
            }
        }

        impl core::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, rhs: $name) -> $name {
                $name(self.0 | rhs.0)
            }
        }

        impl core::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: $name) {
                self.0 |= rhs.0;
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}({self})", stringify!($name))
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let known = Self::NAMED.iter().fold(0, |acc, (flag, _)| acc | flag.0);
                let mut first = true;
                for name in self.names() {
                    if !first {
                        f.write_str(" | ")?;
                    }
                    f.write_str(name)?;
                    first = false;
                }
                let unknown = self.0 & !known;
                if unknown != 0 {
                    if !first {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{unknown:#x}")?;
                    first = false;
                }
                if first {
                    f.write_str("empty")?;
                }
                Ok(())
            }
        }
    };
}
