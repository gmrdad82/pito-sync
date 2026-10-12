use std::fmt;

pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into().into_bytes())
    }

    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }

    fn wipe(&mut self) {
        for byte in self.0.iter_mut() {
            *byte = 0;
        }
        for slot in self.0.spare_capacity_mut() {
            slot.write(0);
        }
        std::hint::black_box(&mut self.0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wiped_secret_is_all_zeroes_up_to_its_capacity() {
        let mut value = String::with_capacity(64);
        value.push_str("s3cr3t-value-0042");
        let mut secret = Secret::new(value);
        assert_eq!(secret.expose(), "s3cr3t-value-0042");
        let capacity = secret.0.capacity();
        secret.wipe();
        assert_eq!(secret.0.len(), 17);
        assert_eq!(secret.0.capacity(), capacity);
        assert!(secret.0.iter().all(|&b| b == 0));
        assert!(!secret.expose().contains("s3cr3t"));
    }
}
