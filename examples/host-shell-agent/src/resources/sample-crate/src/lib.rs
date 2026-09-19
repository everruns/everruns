//! Split a slice into fixed-size chunks.

/// How many chunks of `size` elements `len` elements make.
pub fn chunk_count(len: usize, size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    len / size
}

/// Split `items` into chunks of at most `size` elements.
pub fn chunks<T: Clone>(items: &[T], size: usize) -> Vec<Vec<T>> {
    (0..chunk_count(items.len(), size))
        .map(|index| {
            let start = index * size;
            items[start..(start + size).min(items.len())].to_vec()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_final_chunk_still_counts() {
        assert_eq!(chunk_count(9, 3), 3);
        assert_eq!(chunk_count(10, 3), 4);
        assert_eq!(chunk_count(0, 3), 0);
    }

    #[test]
    fn every_element_lands_in_exactly_one_chunk() {
        let items: Vec<u8> = (1..=10).collect();
        let chunks = chunks(&items, 3);

        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[3], vec![10]);
        assert_eq!(
            chunks.concat(),
            items,
            "chunking must not lose or duplicate elements"
        );
    }
}
