use rand::Rng;

const VALID_UID_CHARACTERS: &str = "!#%()*+,-./:;=?@[]^_`{|}~ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

const PALETTE_UIDS: &[&str] = &["while", "timer", "of", "movex", "movey", "setx", "sety"];

pub fn numeric_to_str_uid(n: u64) -> String {
    let base = VALID_UID_CHARACTERS.len() as u64;
    if n == 0 {
        return VALID_UID_CHARACTERS[0..1].to_string();
    }
    let mut digits = Vec::new();
    let mut n = n;
    while n > 0 {
        digits.push(VALID_UID_CHARACTERS.chars().nth((n % base) as usize).unwrap());
        n /= base;
    }
    digits.reverse();
    digits.into_iter().collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct UidGenerator {
    generated_ids: u64,
    minify: bool,
}

impl UidGenerator {
    pub fn new(minify: bool) -> Self {
        UidGenerator {
            generated_ids: 0,
            minify,
        }
    }

    pub fn gen_id(&mut self) -> String {
        if !self.minify {
            let mut rng = rand::rng();
            let bytes: [u8; 16] = rng.random();
            hex::encode(bytes)
        } else {
            loop {
                let id = numeric_to_str_uid(self.generated_ids);
                self.generated_ids += 1;
                if !PALETTE_UIDS.contains(&id.as_str()) {
                    return id;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_to_str_uid_zero() {
        assert_eq!(numeric_to_str_uid(0), "!");
    }

    #[test]
    fn test_numeric_to_str_uid_one() {
        assert_eq!(numeric_to_str_uid(1), "#");
    }

    #[test]
    fn test_numeric_to_str_uid_small() {
        let result = numeric_to_str_uid(10);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_uid_generator_minify() {
        let mut generator = UidGenerator::new(true);
        let id1 = generator.gen_id();
        let id2 = generator.gen_id();
        assert_ne!(id1, id2);
        assert!(!PALETTE_UIDS.contains(&id1.as_str()));
        assert!(!PALETTE_UIDS.contains(&id2.as_str()));
    }

    #[test]
    fn test_uid_generator_non_minify() {
        let mut generator = UidGenerator::new(false);
        let id1 = generator.gen_id();
        let id2 = generator.gen_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 32);
        assert_eq!(id2.len(), 32);
    }
}