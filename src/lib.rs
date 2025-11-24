mod people;

use people::FAMOUS_PEOPLE;
use fastrand;

pub fn random_name() -> String {
    return fastrand::choice(FAMOUS_PEOPLE).unwrap().to_string();
}