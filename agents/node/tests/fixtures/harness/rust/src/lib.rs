pub fn add_two(value: i32) -> i32 {
    value + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two() {
        assert_eq!(add_two(40), 42);
    }
}
