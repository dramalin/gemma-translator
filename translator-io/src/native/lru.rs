/// Keeps at most `capacity` values, dropping the least recently used one
/// before a new value is created (models are large; never hold three).
pub struct LruCache<K, V> {
    capacity: usize,
    /// Most recently used last.
    entries: Vec<(K, V)>,
}

impl<K: PartialEq + Copy, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        LruCache {
            capacity,
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Returns the value for `key`, creating it on a miss. On a miss `check`
    /// runs first, so a value that cannot be created (missing model files)
    /// evicts nothing; then the least recently used value is dropped if the
    /// cache is full, and only then does `create` run.
    pub fn get_or_try_insert_with<E>(
        &mut self,
        key: K,
        check: impl FnOnce() -> Result<(), E>,
        create: impl FnOnce() -> Result<V, E>,
    ) -> Result<&mut V, E> {
        if let Some(position) = self
            .entries
            .iter()
            .position(|(existing, _)| *existing == key)
        {
            let entry = self.entries.remove(position);
            self.entries.push(entry);
        } else {
            check()?;
            if self.entries.len() >= self.capacity {
                self.entries.remove(0);
            }
            let value = create()?;
            self.entries.push((key, value));
        }
        let (_, value) = self.entries.last_mut().expect("an entry was just pushed");
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    fn passes() -> Result<(), ()> {
        Ok(())
    }

    struct Recorded(&'static str, Rc<RefCell<Vec<&'static str>>>);

    impl Drop for Recorded {
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }

    #[test]
    fn evicts_the_least_recently_used_entry_before_creating() {
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let mut cache = LruCache::new(2);
        let make = |name| {
            let dropped = Rc::clone(&dropped);
            move || Ok::<_, ()>(Recorded(name, dropped))
        };
        cache
            .get_or_try_insert_with("a", passes, make("a"))
            .unwrap();
        cache
            .get_or_try_insert_with("b", passes, make("b"))
            .unwrap();
        cache
            .get_or_try_insert_with("a", passes, make("unused"))
            .unwrap();
        assert!(dropped.borrow().is_empty());
        let created_c = cache
            .get_or_try_insert_with("c", passes, || {
                // "b" (least recently used) must already be gone: two models never
                // share memory with a third during a load on the Pi.
                assert_eq!(*dropped.borrow(), ["b"]);
                make("c")()
            })
            .unwrap();
        assert_eq!(created_c.0, "c");
        assert_eq!(
            cache
                .get_or_try_insert_with("a", passes, make("again"))
                .unwrap()
                .0,
            "a"
        );
    }

    #[test]
    fn failed_check_evicts_nothing() {
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let mut cache = LruCache::new(2);
        for name in ["a", "b"] {
            let dropped = Rc::clone(&dropped);
            cache
                .get_or_try_insert_with(
                    name,
                    || Ok::<_, &str>(()),
                    move || Ok(Recorded(name, dropped)),
                )
                .unwrap();
        }
        let missing = cache.get_or_try_insert_with(
            "c",
            || Err("c is missing"),
            || -> Result<Recorded, &str> { panic!("created despite a failed check") },
        );
        assert_eq!(missing.err(), Some("c is missing"));
        assert!(dropped.borrow().is_empty());
        for name in ["a", "b"] {
            let cached = cache.get_or_try_insert_with(
                name,
                || -> Result<(), &str> { panic!("checked a cached entry") },
                || panic!("{name} was evicted"),
            );
            assert_eq!(cached.unwrap().0, name);
        }
    }

    #[test]
    fn failed_creation_leaves_no_entry() {
        let mut cache: LruCache<u8, u8> = LruCache::new(2);
        assert_eq!(
            cache.get_or_try_insert_with(1, || Ok(()), || Err("nope")),
            Err("nope")
        );
        assert_eq!(
            cache.get_or_try_insert_with(1, || Ok(()), || Ok::<_, &str>(7)),
            Ok(&mut 7)
        );
    }
}
