//! Validate every serialized float, including nested AI and movement state.
//! Keeping this at the serialization boundary prevents private nested state from
//! accidentally escaping finite checks when another persistent field is added.
use serde::ser::{self, Serialize};

#[derive(Debug)]
pub(super) struct Error;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Grand checkpoint contains a nonfinite or unbounded number")
    }
}
impl std::error::Error for Error {}
impl ser::Error for Error {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self
    }
}

pub(super) struct Finite;
macro_rules! primitive { ($($name:ident: $ty:ty),*) => { $(fn $name(self, _: $ty) -> Result<(), Error> { Ok(()) })* }; }
impl ser::Serializer for Finite {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;
    primitive!(serialize_bool: bool, serialize_i8: i8, serialize_i16: i16, serialize_i32: i32, serialize_i64: i64, serialize_i128: i128, serialize_u8: u8, serialize_u16: u16, serialize_u32: u32, serialize_u64: u64, serialize_u128: u128, serialize_char: char, serialize_str: &str, serialize_bytes: &[u8]);
    fn serialize_f32(self, value: f32) -> Result<(), Error> {
        self.serialize_f64(f64::from(value))
    }
    fn serialize_f64(self, value: f64) -> Result<(), Error> {
        if value.is_finite() && value.abs() < 1.0e9 {
            Ok(())
        } else {
            Err(Error)
        }
    }
    fn serialize_none(self) -> Result<(), Error> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Error> {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Error> {
        Ok(self)
    }
}
macro_rules! compound { ($trait:ident, $method:ident $(, $key:ident: $keyty:ty)?) => {
    impl ser::$trait for Finite { type Ok = (); type Error = Error;
        fn $method<T: ?Sized + Serialize>(&mut self, $($key: $keyty,)? value: &T) -> Result<(), Error> { $(let _ = $key;)? value.serialize(Finite) }
        fn end(self) -> Result<(), Error> { Ok(()) }
    }
}; }
compound!(SerializeSeq, serialize_element);
compound!(SerializeTuple, serialize_element);
compound!(SerializeTupleStruct, serialize_field);
compound!(SerializeTupleVariant, serialize_field);
compound!(SerializeStruct, serialize_field, key: &'static str);
compound!(SerializeStructVariant, serialize_field, key: &'static str);
impl ser::SerializeMap for Finite {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(Finite)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(Finite)
    }
    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}
