use subset::{SubsetBuilder, Subset};
use delta::Delta;
use rope::{Rope, RopeInfo};

pub fn find_deletions(substr: &str, s: &str) -> Subset {
    let mut sb = SubsetBuilder::new();
    let mut j = 0;
    for i in 0..s.len() {
        if j < substr.len() && substr.as_bytes()[j] == s.as_bytes()[i] {
            j += 1;
        } else {
            sb.add_range(i, i + 1);
        }
    }
    sb.build()
}

impl Delta<RopeInfo> {
    pub fn apply_to_string(&self, s: &str) -> String {
        String::from(self.apply(&Rope::from(s)))
    }
}
