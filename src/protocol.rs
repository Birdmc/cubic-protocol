use std::f32::consts::PI;
use std::marker::PhantomData;
use crate::bytes::{InputByteQueue, InputByteQueueError, OutputByteQueue};
use std::mem;
use std::str::{from_utf8, Utf8Error};
use cubic_chat::component::ComponentType;
use cubic_chat::identifier::{Identifier, IdentifierError};
use euclid::default::Vector3D;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Error;
use uuid::Uuid;

#[derive(Debug)]
pub enum WriteError {
    JSON(serde_json::Error),
    NBT(fastnbt::error::Error),
}

#[derive(Debug)]
pub enum ReadError {
    BadVarNum,
    BadUtf8(Utf8Error),
    BadEntityDataType(i32, i32),
    BadEntityDataIndex(u8),
    BadEnumValue,
    BadStringLimit(i32),
    BadIdentifier(IdentifierError),
    BadJson(serde_json::Error),
    InputQueue(InputByteQueueError),
}

impl From<serde_json::Error> for WriteError {
    fn from(val: Error) -> Self {
        WriteError::JSON(val)
    }
}

impl From<fastnbt::error::Error> for WriteError {
    fn from(val: fastnbt::error::Error) -> Self {
        WriteError::NBT(val)
    }
}

impl From<Utf8Error> for ReadError {
    fn from(err: Utf8Error) -> Self {
        ReadError::BadUtf8(err)
    }
}

impl From<IdentifierError> for ReadError {
    fn from(err: IdentifierError) -> Self {
        ReadError::BadIdentifier(err)
    }
}

impl From<serde_json::Error> for ReadError {
    fn from(err: Error) -> Self {
        ReadError::BadJson(err)
    }
}

impl From<InputByteQueueError> for ReadError {
    fn from(err: InputByteQueueError) -> Self {
        ReadError::InputQueue(err)
    }
}

macro_rules! delegate_type {
    ($name: ident, $delegates: ty) => {
        #[derive(Copy, Clone, Debug, Default, PartialEq)]
        pub struct $name(pub $delegates);

        impl From<$delegates> for $name {
            fn from(val: $delegates) -> Self {
                $name(val)
            }
        }

        impl From<$name> for $delegates {
            fn from(val: $name) -> Self {
                val.0
            }
        }
    }
}

macro_rules! fully_delegate_type {
    ($name: ident, $delegates: ty) => {
        delegate_type!($name, $delegates);

        impl Writable for $name {
            fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
                self.0.write(output)
            }
        }

        #[async_trait::async_trait]
        impl Readable for $name {
            async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
                Ok(<$delegates as Readable>::read(input).await?.into())
            }
        }
    }
}

macro_rules! protocol_num_type {
    ($type: ident) => {
        impl Writable for $type {
            fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
                output.put_bytes(&self.to_le_bytes());
                Ok(())
            }
        }

        #[async_trait::async_trait]
        impl Readable for $type {
            async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
                let mut bytes = [0_u8; mem::size_of::<$type>()];
                input.take_bytes(&mut bytes).await?;
                Ok($type::from_le_bytes(bytes))
            }
        }
    }
}

macro_rules! protocol_var_num_type {
    ($type: ident, $num_type: ident, $num_unsigned: ident) => {
        impl Writable for $type {
            fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError>{
                let mut value = $num_type::from(*self) as $num_unsigned;
                loop {
                    if ((value & !0x7f) == 0) {
                        output.put_byte(value as u8);
                        break;
                    }
                    output.put_byte(((value as u8) & 0x7f) | 0x80);
                    value >>= 7;
                }
                Ok(())
            }
        }

        #[async_trait::async_trait] impl Readable for $type {
            async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
                const BITS: $num_type = (mem::size_of::<$num_type>() * 8) as $num_type;
                let mut value: $num_type = 0;
                let mut position: $num_type = 0;
                loop {
                    let current_byte = input.take_byte().await? as $num_type;
                    value |= (current_byte & 0x7f) << position;
                    if ((current_byte & 0x80) == 0) {
                        break;
                    }
                    position += 7;
                    if (position >= BITS) {
                        return Err(ReadError::BadVarNum)
                    }
                }
                Ok($type::from(value))
            }
        }

        impl SizeNumber for $type {
            fn as_size(&self) -> usize {
                self.0 as usize
            }

            fn new_from_size(size: usize) -> Self {
                $type::from(size as $num_type)
            }
        }
    }
}

pub trait Writable {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError>;
}

#[async_trait::async_trait]
pub trait Readable: Sized {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError>;
}

impl<T> Writable for PhantomData<T> {
    fn write(&self, _: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T> Readable for PhantomData<T> {
    async fn read(_: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        Ok(PhantomData)
    }
}

impl Writable for u8 {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        output.put_byte(*self);
        Ok(())
    }
}

#[async_trait::async_trait]
impl Readable for u8 {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        Ok(input.take_byte().await?)
    }
}

impl Writable for i8 {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        (*self as u8).write(output)
    }
}

#[async_trait::async_trait]
impl Readable for i8 {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        u8::read(input).await.map(|val| val as i8)
    }
}

impl Writable for bool {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        match self {
            true => 1_u8,
            false => 0_u8,
        }.write(output)
    }
}

#[async_trait::async_trait]
impl Readable for bool {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        u8::read(input).await.map(|val| val != 0)
    }
}

protocol_num_type!(i16);
protocol_num_type!(u16);
protocol_num_type!(i32);
protocol_num_type!(u32);
protocol_num_type!(i64);
protocol_num_type!(u64);
protocol_num_type!(i128);
protocol_num_type!(u128);
protocol_num_type!(f32);
protocol_num_type!(f64);

delegate_type!(VarInt, i32);
delegate_type!(VarLong, i64);
protocol_var_num_type!(VarInt, i32, u32);
protocol_var_num_type!(VarLong, i64, u64);

#[async_trait::async_trait]
impl Readable for Uuid {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let mut bytes = [0_u8; 16];
        input.take_bytes(&mut bytes).await?;
        Ok(Uuid::from_bytes(bytes))
    }
}

impl Writable for Uuid {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        output.put_bytes(self.as_bytes());
        Ok(())
    }
}

const STRING_LIMIT: i32 = 32767;
const CHAT_LIMIT: i32 = 262144;

async fn read_string_with_limit(input: &mut impl InputByteQueue, limit: i32) -> Result<String, ReadError> {
    let length: i32 = VarInt::read(input).await?.into();
    match length > limit {
        true => Err(ReadError::BadStringLimit(limit)),
        false => {
            let mut vec = Vec::with_capacity(length as usize);
            input.take_vec(length as usize, &mut vec).await?;
            Ok(from_utf8(vec.as_slice())?.into())
        }
    }
}

#[async_trait::async_trait]
impl Readable for String {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        read_string_with_limit(input, STRING_LIMIT).await
    }
}

impl Writable for String {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        VarInt(self.len() as i32).write(output)?;
        output.put_bytes(self.as_bytes());
        Ok(())
    }
}

#[async_trait::async_trait]
impl Readable for Identifier {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let str = String::read(input).await?;
        Ok(Identifier::from_full(str)?)
    }
}

impl Writable for Identifier {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        self.to_string().write(output)
    }
}

#[async_trait::async_trait]
impl Readable for ComponentType {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let str = read_string_with_limit(input, CHAT_LIMIT).await?;
        Ok(serde_json::from_str(&*str)?)
    }
}

impl Writable for ComponentType {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        let str = serde_json::to_string(self)?;
        str.write(output)
    }
}

pub type BlockPosition = Vector3D<i32>;

const BLOCK_X_MASK: u64 = 0x3ffffff;
const BLOCK_Z_MASK: u64 = BLOCK_X_MASK;
const BLOCK_Y_MASK: u64 = 0xfff;

const BLOCK_X_NEG_BOUND: i32 = 1 << 25;
const BLOCK_Z_NEG_BOUND: i32 = BLOCK_X_NEG_BOUND;
const BLOCK_Y_NEG_BOUND: i32 = 1 << 11;

#[async_trait::async_trait]
impl Readable for BlockPosition {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let val = u64::read(input).await?;
        let mut x = (val >> 38) as i32;
        let mut z = ((val >> 12) & BLOCK_Z_MASK) as i32;
        let mut y = (val & BLOCK_Y_MASK) as i32;
        if x >= BLOCK_X_NEG_BOUND {
            x -= BLOCK_X_NEG_BOUND << 1;
        }
        if z >= BLOCK_Z_NEG_BOUND {
            z -= BLOCK_Z_NEG_BOUND << 1;
        }
        if y >= BLOCK_Y_NEG_BOUND {
            y -= BLOCK_Y_NEG_BOUND << 1;
        }
        Ok(BlockPosition::new(x, y, z))
    }
}

impl Writable for BlockPosition {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        let x = self.x as i64;
        let z = self.z as i64;
        let y = self.y as i64;
        (((x & BLOCK_X_MASK as i64) << 38) |
            ((z & BLOCK_Z_MASK as i64) << 12) |
            (y & BLOCK_Y_MASK as i64)
        ).write(output)
    }
}

#[async_trait::async_trait]
impl<T: Readable> Readable for Option<T> {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let provided = bool::read(input).await?;
        Ok(match provided {
            true => Some(T::read(input).await?),
            false => None
        })
    }
}

impl<T: Writable> Writable for Option<T> {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        match self {
            None => false.write(output),
            Some(val) => {
                true.write(output)?;
                val.write(output)
            }
        }
    }
}

#[derive(Clone, PartialOrd, PartialEq, Debug)]
pub struct RemainingBytesArray<T> {
    pub result: Vec<T>,
}

#[derive(Clone, PartialOrd, PartialEq, Debug)]
pub struct LengthProvidedArray<T, L> {
    pub result: Vec<T>,
    size: PhantomData<L>,
}

impl<T> RemainingBytesArray<T> {
    pub fn new(result: Vec<T>) -> RemainingBytesArray<T> {
        RemainingBytesArray { result }
    }
}

impl<T> From<Vec<T>> for RemainingBytesArray<T> {
    fn from(v: Vec<T>) -> Self {
        RemainingBytesArray::new(v)
    }
}

impl<T> From<RemainingBytesArray<T>> for Vec<T> {
    fn from(arr: RemainingBytesArray<T>) -> Self {
        arr.result
    }
}

#[async_trait::async_trait]
impl<T: Readable + Send + Sync> Readable for RemainingBytesArray<T> {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let mut result = Vec::new();
        while input.has_bytes(1) {
            result.push(T::read(input).await?)
        }
        Ok(RemainingBytesArray {
            result
        })
    }
}

impl<T: Writable> Writable for RemainingBytesArray<T> {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        for element in &self.result {
            element.write(output)?
        }
        Ok(())
    }
}

pub trait SizeNumber {
    fn as_size(&self) -> usize;

    fn new_from_size(size: usize) -> Self;
}

macro_rules! primitive_size_number {
    ($type: ident) => {
        impl SizeNumber for $type {
            fn as_size(&self) -> usize {
                *self as usize
            }

            fn new_from_size(size: usize) -> Self {
                size as $type
            }
        }
    }
}

primitive_size_number!(usize);
primitive_size_number!(u128);
primitive_size_number!(i128);
primitive_size_number!(u64);
primitive_size_number!(i64);
primitive_size_number!(u32);
primitive_size_number!(i32);
primitive_size_number!(u16);
primitive_size_number!(i16);
primitive_size_number!(u8);
primitive_size_number!(i8);

impl<T, L> LengthProvidedArray<T, L> {
    pub fn new(result: Vec<T>) -> LengthProvidedArray<T, L> {
        LengthProvidedArray { result, size: PhantomData }
    }
}

impl<T, L> From<Vec<T>> for LengthProvidedArray<T, L> {
    fn from(v: Vec<T>) -> Self {
        LengthProvidedArray::new(v)
    }
}

impl<T, L> From<LengthProvidedArray<T, L>> for Vec<T> {
    fn from(arr: LengthProvidedArray<T, L>) -> Self {
        arr.result
    }
}

#[async_trait::async_trait]
impl<T: Readable + Send + Sync, L: Readable + SizeNumber> Readable for LengthProvidedArray<T, L> {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let size = L::read(input).await?.as_size();
        let mut result = Vec::new();
        for _ in 0..size {
            result.push(T::read(input).await?);
        }
        Ok(LengthProvidedArray::new(result))
    }
}

impl<T: Writable, L: Writable + SizeNumber> Writable for LengthProvidedArray<T, L> {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        L::new_from_size(self.result.len()).write(output)?;
        for element in &self.result {
            element.write(output)?;
        }
        Ok(())
    }
}

delegate_type!(Angle, f32);

#[async_trait::async_trait]
impl Readable for Angle {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        Ok(Angle::from(
            (u8::read(input).await? as f32) * PI / 256.0
        ))
    }
}

impl Writable for Angle {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        (self.0 * 256.0 / PI).write(output)
    }
}

#[derive(Debug)]
pub struct ProtocolJson<T: Sized> {
    pub value: T,
}

impl<T: Serialize> Writable for ProtocolJson<T> {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        serde_json::to_string(&self.value)?.write(output)
    }
}

#[async_trait::async_trait]
impl<T: DeserializeOwned> Readable for ProtocolJson<T> {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        let vec = LengthProvidedArray::<u8, VarInt>::read(input).await?.result;
        Ok(ProtocolJson {
            value: serde_json::from_slice(vec.as_slice())?
        })
    }
}

impl<T> From<T> for ProtocolJson<T> {
    fn from(value: T) -> Self {
        ProtocolJson { value }
    }
}

delegate_type!(Rotation, Vector3D<f32>);

impl Writable for Rotation {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        self.0.x.write(output)?;
        self.0.y.write(output)?;
        self.0.z.write(output)
    }
}

#[async_trait::async_trait]
impl Readable for Rotation {
    async fn read(input: &mut impl InputByteQueue) -> Result<Self, ReadError> {
        Ok(Vector3D::new(
            f32::read(input).await?,
            f32::read(input).await?,
            f32::read(input).await?,
        ).into())
    }
}

fully_delegate_type!(BlockId, VarInt);

impl Writable for fastnbt::Value {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        RemainingBytesArray::new(fastnbt::to_bytes(self)?).write(output)
    }
}

#[derive(Debug)]
pub struct Slot {
    pub item_id: VarInt,
    pub item_count: u8,
    pub nbt: fastnbt::Value,
}

impl Writable for Slot {
    fn write(&self, output: &mut impl OutputByteQueue) -> Result<(), WriteError> {
        self.item_id.write(output)?;
        self.item_count.write(output)?;
        self.nbt.write(output)
    }
}

#[cfg(all(test, feature = "tokio-bytes"))]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};
    use cubic_chat::color::DefaultColor;
    use cubic_chat::component::{TextComponent};
    use crate::tokio::{BytesInputQueue, BytesOutputQueue};

    macro_rules! test_macro {
        ($($type: ident => $value: expr)*) => {
            let mut output = BytesOutputQueue::new();
            $($value.write(&mut output).unwrap();)*
            let mut input = BytesInputQueue::new_without_slice(output.get_bytes());
            $(assert_eq!($type::read(&mut input).await.unwrap(), $value);)*
        }
    }

    #[actix_rt::test]
    async fn success_integer_test() {
        {
            let mut output = BytesOutputQueue::new();
            0xff321233_u32.write(&mut output).unwrap();
            let bytes = output.get_bytes();
            assert_eq!(bytes.len(), 4);
            assert_eq!(bytes[0], 0x33);
            assert_eq!(bytes[1], 0x12);
            assert_eq!(bytes[2], 0x32);
            assert_eq!(bytes[3], 0xff);
        }
        {
            let mut bytes = BytesMut::new();
            bytes.put_u8(0x97);
            bytes.put_u8(0x32);
            bytes.put_u8(0x11);
            bytes.put_u8(0xaa);
            let mut input = BytesInputQueue::new(4, bytes);
            assert_eq!(u32::read(&mut input).await.unwrap(), 0xaa113297_u32);
        }
        test_macro! {
            i64 => 321953285_i64
            u64 => 32194325340956843_u64
            i32 => 329853_i32
            u32 => 2035936534_u32
            i16 => 32474_i16
            u16 => 6886_u16
            u8 => 244_u8
            i8 => -120_i8
            bool => true
            bool => false
        }
    }

    #[actix_rt::test]
    async fn success_var_num_test() {
        {
            let mut output = BytesOutputQueue::new();
            VarInt(0).write(&mut output).unwrap();
            let bytes = output.get_bytes();
            assert_eq!(bytes[0], 0);
        }
        {
            let mut output = BytesOutputQueue::new();
            VarInt(2097151).write(&mut output).unwrap();
            let bytes = output.get_bytes();
            assert_eq!(bytes.to_vec(), vec![255, 255, 127]);
        }
        {
            let mut output = BytesOutputQueue::new();
            VarInt(-1).write(&mut output).unwrap();
            let bytes = output.get_bytes();
            assert_eq!(bytes.to_vec(), vec![255, 255, 255, 255, 15]);
        }
        {
            let mut input = BytesInputQueue::new(
                1, BytesMut::from_iter(vec![0]),
            );
            assert_eq!(VarInt::read(&mut input).await.unwrap(), VarInt(0));
        }
        {
            let mut input = BytesInputQueue::new(
                3, BytesMut::from_iter(vec![255, 255, 127]),
            );
            assert_eq!(VarInt::read(&mut input).await.unwrap(), VarInt(2097151));
        }
        {
            let mut input = BytesInputQueue::new(
                5, BytesMut::from_iter(vec![255, 255, 255, 255, 15]),
            );
            assert_eq!(VarInt::read(&mut input).await.unwrap(), VarInt(-1));
        }
        {
            let mut output = BytesOutputQueue::new();
            VarLong(-1).write(&mut output).unwrap();
            assert_eq!(output.get_bytes().to_vec(), vec![255, 255, 255, 255, 255, 255, 255, 255, 255, 1]);
        }
        {
            let mut output = BytesOutputQueue::new();
            VarLong(2147483647).write(&mut output).unwrap();
            assert_eq!(output.get_bytes().to_vec(), vec![255, 255, 255, 255, 7]);
        }
        {
            let mut output = BytesOutputQueue::new();
            VarLong(-2147483648).write(&mut output).unwrap();
            assert_eq!(output.get_bytes().to_vec(), vec![128, 128, 128, 128, 248, 255, 255, 255, 255, 1])
        }
        test_macro!(
            VarInt => VarInt(32)
            VarInt => VarInt(-1)
            VarLong => VarLong(-3231)
            VarLong => VarLong(-32)
            VarLong => VarLong(32186094)
            VarLong => VarLong(-321936749532)
        );
    }

    #[actix_rt::test]
    async fn success_string_test() {
        test_macro! {
            String => "jenya705 is the best".to_string()
        }
        // test for not ascii characters
        test_macro! {
            String => "я самый лучший".to_string()
        }
    }

    #[actix_rt::test]
    async fn success_block_position_test() {
        test_macro! {
            BlockPosition => BlockPosition::new(1, 1, 1)
            BlockPosition => BlockPosition::new(-321, 32, -32)
            BlockPosition => BlockPosition::new(-320, 32, -3123)
            BlockPosition => BlockPosition::new(321, 255, -325837)
        }
    }

    #[actix_rt::test]
    async fn success_chat_test() {
        test_macro! {
            ComponentType => ComponentType::Text({
                let mut component = TextComponent::new("hi!".into());
                component.base.bold = Some(true);
                component.base.extra.push({
                    let mut component = TextComponent::new("bye!".into());
                    component.base.color = Some(DefaultColor::Red.into());
                    component.into()
                });
                component
            })
        }
    }

    #[actix_rt::test]
    async fn success_remaining_bytes_array_test() {
        test_macro! {
            RemainingBytesArray => RemainingBytesArray::new(vec![32_u8, 233_u8, 211_u8])
        }
        test_macro! {
            RemainingBytesArray => RemainingBytesArray::new(vec![VarInt(-1), VarInt(325), VarInt(3219538)])
        }
    }

    #[actix_rt::test]
    async fn success_length_provided_array_test() {
        test_macro! {
            LengthProvidedArray => LengthProvidedArray::<VarInt, i16>::new(
                vec![VarInt(321), VarInt(-312325), VarInt(328138)]
            )
            LengthProvidedArray => LengthProvidedArray::<String, VarInt>::new(
                vec!["hello!".to_string(), "poka".to_string(), "dadada".to_string()]
            )
        }
    }

    #[actix_rt::test]
    async fn success_compound_test() {
        test_macro!(
            u32 => 0_u32
            u16 => 12_u16
            u8 => 32_u8
            VarInt => VarInt(-1)
            VarLong => VarLong(32)
            VarLong => VarLong(-2)
            String => "some_string".to_string()
            BlockPosition => BlockPosition::new(32321, 233, -32325)
        );
    }
}