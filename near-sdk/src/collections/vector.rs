//! A vector implemented on a trie. Unlike standard vector does not support insertion and removal
//! of an element results in the last element being placed in the empty position.
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::collections::append_slice;
use crate::{env, IntoStorageKey};

const ERR_INCONSISTENT_STATE: &str = "The collection is an inconsistent state. Did previous smart contract execution terminate unexpectedly?";
const ERR_ELEMENT_DESERIALIZATION: &str = "Cannot deserialize element";
const ERR_ELEMENT_SERIALIZATION: &str = "Cannot serialize element";
const ERR_INDEX_OUT_OF_BOUNDS: &str = "Index out of bounds";

fn expect_consistent_state<T>(val: Option<T>) -> T {
    val.unwrap_or_else(|| env::panic_str(ERR_INCONSISTENT_STATE))
}

/// An iterable implementation of vector that stores its content on the trie.
/// Uses the following map: index -> element.
#[derive(BorshSerialize, BorshDeserialize)]
#[cfg_attr(not(feature = "expensive-debug"), derive(Debug))]
pub struct Vector<T> {
    len: u64,
    prefix: Vec<u8>,
    #[borsh_skip]
    el: PhantomData<T>,
}

impl<T> Vector<T> {
    /// Returns the number of elements in the vector, also referred to as its size.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns `true` if the vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Create new vector with zero elements. Use `id` as a unique identifier on the trie.
    pub fn new<S>(prefix: S) -> Self
    where
        S: IntoStorageKey,
    {
        Self { len: 0, prefix: prefix.into_storage_key(), el: PhantomData }
    }

    fn index_to_lookup_key(&self, index: u64) -> Vec<u8> {
        append_slice(&self.prefix, &index.to_le_bytes()[..])
    }

    /// Returns the serialized element by index or `None` if it is not present.
    pub fn get_raw(&self, index: u64) -> Option<Vec<u8>> {
        if index >= self.len {
            return None;
        }
        let lookup_key = self.index_to_lookup_key(index);
        Some(expect_consistent_state(env::storage_read(&lookup_key)))
    }

    /// Removes an element from the vector and returns it in serialized form.
    /// The removed element is replaced by the last element of the vector.
    /// Does not preserve ordering, but is `O(1)`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn swap_remove_raw(&mut self, index: u64) -> Vec<u8> {
        if index >= self.len {
            env::panic_str(ERR_INDEX_OUT_OF_BOUNDS)
        } else if index + 1 == self.len {
            expect_consistent_state(self.pop_raw())
        } else {
            let lookup_key = self.index_to_lookup_key(index);
            let raw_last_value = self.pop_raw().expect("checked `index < len` above, so `len > 0`");
            if env::storage_write(&lookup_key, &raw_last_value) {
                expect_consistent_state(env::storage_get_evicted())
            } else {
                env::panic_str(ERR_INCONSISTENT_STATE)
            }
        }
    }

    /// Appends a serialized element to the back of the collection.
    pub fn push_raw(&mut self, raw_element: &[u8]) {
        let lookup_key = self.index_to_lookup_key(self.len);
        self.len += 1;
        env::storage_write(&lookup_key, raw_element);
    }

    /// Removes the last element from a vector and returns it without deserializing, or `None` if it is empty.
    pub fn pop_raw(&mut self) -> Option<Vec<u8>> {
        if self.is_empty() {
            None
        } else {
            let last_index = self.len - 1;
            let last_lookup_key = self.index_to_lookup_key(last_index);

            self.len -= 1;
            let raw_last_value = if env::storage_remove(&last_lookup_key) {
                expect_consistent_state(env::storage_get_evicted())
            } else {
                env::panic_str(ERR_INCONSISTENT_STATE)
            };
            Some(raw_last_value)
        }
    }

    /// Inserts a serialized element at `index`, returns a serialized evicted element.
    ///
    /// # Panics
    ///
    /// If `index` is out of bounds.
    pub fn replace_raw(&mut self, index: u64, raw_element: &[u8]) -> Vec<u8> {
        if index >= self.len {
            env::panic_str(ERR_INDEX_OUT_OF_BOUNDS)
        } else {
            let lookup_key = self.index_to_lookup_key(index);
            if env::storage_write(&lookup_key, raw_element) {
                expect_consistent_state(env::storage_get_evicted())
            } else {
                env::panic_str(ERR_INCONSISTENT_STATE);
            }
        }
    }

    /// Iterate over raw serialized elements.
    pub fn iter_raw(&self) -> impl Iterator<Item = Vec<u8>> + '_ {
        (0..self.len).map(move |i| {
            let lookup_key = self.index_to_lookup_key(i);
            expect_consistent_state(env::storage_read(&lookup_key))
        })
    }

    /// Extends vector from the given collection of serialized elements.
    pub fn extend_raw<IT: IntoIterator<Item = Vec<u8>>>(&mut self, iter: IT) {
        for el in iter {
            self.push_raw(&el)
        }
    }
}

impl<T> Vector<T> {
    /// Removes all elements from the collection.
    pub fn clear(&mut self) {
        for i in 0..self.len {
            let lookup_key = self.index_to_lookup_key(i);
            env::storage_remove(&lookup_key);
        }
        self.len = 0;
    }
}

impl<T> Vector<T>
where
    T: BorshSerialize,
{
    fn serialize_element(element: &T) -> Vec<u8> {
        element.try_to_vec().unwrap_or_else(|_| env::panic_str(ERR_ELEMENT_SERIALIZATION))
    }

    /// Appends an element to the back of the collection.
    pub fn push(&mut self, element: &T) {
        let raw_element = Self::serialize_element(element);
        self.push_raw(&raw_element);
    }

    /// Extends vector from the given collection.
    pub fn extend<IT: IntoIterator<Item = T>>(&mut self, iter: IT) {
        for el in iter {
            self.push(&el)
        }
    }
}

impl<T> Vector<T>
where
    T: BorshDeserialize,
{
    fn deserialize_element(raw_element: &[u8]) -> T {
        T::try_from_slice(raw_element)
            .unwrap_or_else(|_| env::panic_str(ERR_ELEMENT_DESERIALIZATION))
    }

    /// Returns the element by index or `None` if it is not present.
    pub fn get(&self, index: u64) -> Option<T> {
        self.get_raw(index).map(|x| Self::deserialize_element(&x))
    }

    /// Removes an element from the vector and returns it.
    /// The removed element is replaced by the last element of the vector.
    /// Does not preserve ordering, but is `O(1)`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn swap_remove(&mut self, index: u64) -> T {
        let raw_evicted = self.swap_remove_raw(index);
        Self::deserialize_element(&raw_evicted)
    }

    /// Removes the last element from a vector and returns it, or `None` if it is empty.
    pub fn pop(&mut self) -> Option<T> {
        self.pop_raw().map(|x| Self::deserialize_element(&x))
    }

    /// Iterate over deserialized elements.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.iter_raw().map(|raw_element| Self::deserialize_element(&raw_element))
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.iter().collect()
    }
}

impl<T> Vector<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    /// Inserts a element at `index`, returns an evicted element.
    ///
    /// # Panics
    ///
    /// If `index` is out of bounds.
    pub fn replace(&mut self, index: u64, element: &T) -> T {
        let raw_element = Self::serialize_element(element);
        Self::deserialize_element(&self.replace_raw(index, &raw_element))
    }
}

#[cfg(feature = "expensive-debug")]
impl<T: std::fmt::Debug + BorshDeserialize> std::fmt::Debug for Vector<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_vec().fmt(f)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use borsh::BorshDeserialize;
    use rand::{Rng, SeedableRng};

    use crate::collections::Vector;

    #[test]
    fn test_push_pop() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(0);
        let mut vec = Vector::new(b"v".to_vec());
        let mut baseline = vec![];
        for _ in 0..500 {
            let value = rng.gen::<u64>();
            vec.push(&value);
            baseline.push(value);
        }
        let actual = vec.to_vec();
        assert_eq!(actual, baseline);
        for _ in 0..1001 {
            assert_eq!(baseline.pop(), vec.pop());
        }
    }

    #[test]
    pub fn test_replace() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(1);
        let mut vec = Vector::new(b"v".to_vec());
        let mut baseline = vec![];
        for _ in 0..500 {
            let value = rng.gen::<u64>();
            vec.push(&value);
            baseline.push(value);
        }
        for _ in 0..500 {
            let index = rng.gen::<u64>() % vec.len();
            let value = rng.gen::<u64>();
            let old_value0 = vec.get(index).unwrap();
            let old_value1 = vec.replace(index, &value);
            let old_value2 = baseline[index as usize];
            assert_eq!(old_value0, old_value1);
            assert_eq!(old_value0, old_value2);
            *baseline.get_mut(index as usize).unwrap() = value;
        }
        let actual = vec.to_vec();
        assert_eq!(actual, baseline);
    }

    #[test]
    pub fn test_swap_remove() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(2);
        let mut vec = Vector::new(b"v".to_vec());
        let mut baseline = vec![];
        for _ in 0..500 {
            let value = rng.gen::<u64>();
            vec.push(&value);
            baseline.push(value);
        }
        for _ in 0..500 {
            let index = rng.gen::<u64>() % vec.len();
            let old_value0 = vec.get(index).unwrap();
            let old_value1 = vec.swap_remove(index);
            let old_value2 = baseline[index as usize];
            let last_index = baseline.len() - 1;
            baseline.swap(index as usize, last_index);
            baseline.pop();
            assert_eq!(old_value0, old_value1);
            assert_eq!(old_value0, old_value2);
        }
        let actual = vec.to_vec();
        assert_eq!(actual, baseline);
    }

    #[test]
    pub fn test_clear() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(3);
        let mut vec = Vector::new(b"v".to_vec());
        for _ in 0..100 {
            for _ in 0..(rng.gen::<u64>() % 20 + 1) {
                let value = rng.gen::<u64>();
                vec.push(&value);
            }
            assert!(!vec.is_empty());
            vec.clear();
            assert!(vec.is_empty());
        }
    }

    #[test]
    pub fn test_extend() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(0);
        let mut vec = Vector::new(b"v".to_vec());
        let mut baseline = vec![];
        for _ in 0..100 {
            let value = rng.gen::<u64>();
            vec.push(&value);
            baseline.push(value);
        }

        for _ in 0..100 {
            let mut tmp = vec![];
            for _ in 0..=(rng.gen::<u64>() % 20 + 1) {
                let value = rng.gen::<u64>();
                tmp.push(value);
            }
            baseline.extend(tmp.clone());
            vec.extend(tmp.clone());
        }
        let actual = vec.to_vec();
        assert_eq!(actual, baseline);
    }

    #[test]
    fn test_debug() {
        let mut rng = rand_xorshift::XorShiftRng::seed_from_u64(4);
        let prefix = b"v".to_vec();
        let mut vec = Vector::new(prefix.clone());
        let mut baseline = vec![];
        for _ in 0..10 {
            let value = rng.gen::<u64>();
            vec.push(&value);
            baseline.push(value);
        }
        let actual = vec.to_vec();
        assert_eq!(actual, baseline);
        for _ in 0..5 {
            assert_eq!(baseline.pop(), vec.pop());
        }
        if cfg!(feature = "expensive-debug") {
            assert_eq!(format!("{:#?}", vec), format!("{:#?}", baseline));
        } else {
            assert_eq!(
                format!("{:?}", vec),
                format!("Vector {{ len: 5, prefix: {:?}, el: PhantomData }}", vec.prefix)
            );
        }

        #[derive(Debug, BorshDeserialize)]
        struct WithoutBorshSerialize(u64);

        let deserialize_only_vec =
            Vector::<WithoutBorshSerialize> { len: vec.len(), prefix, el: Default::default() };
        let baseline: Vec<_> = baseline.into_iter().map(WithoutBorshSerialize).collect();
        if cfg!(feature = "expensive-debug") {
            assert_eq!(format!("{:#?}", deserialize_only_vec), format!("{:#?}", baseline));
        } else {
            assert_eq!(
                format!("{:?}", deserialize_only_vec),
                format!(
                    "Vector {{ len: 5, prefix: {:?}, el: PhantomData }}",
                    deserialize_only_vec.prefix
                )
            );
        }
    }
}
