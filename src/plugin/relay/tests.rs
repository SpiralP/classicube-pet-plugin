use super::RelayMessage;

fn round_trip(msg: &RelayMessage) -> RelayMessage {
    let bytes = borsh::to_vec(msg).unwrap();
    borsh::from_slice::<RelayMessage>(&bytes).unwrap()
}

#[test]
fn hello_round_trip() {
    let msg = RelayMessage::Hello {
        version: "1.2.3".to_string(),
    };
    let rt = round_trip(&msg);
    let bytes_orig = borsh::to_vec(&msg).unwrap();
    let bytes_rt = borsh::to_vec(&rt).unwrap();
    assert_eq!(bytes_orig, bytes_rt);
}

#[test]
fn pet_state_round_trip() {
    let msg = RelayMessage::PetState {
        model: "humanoid".to_string(),
        model_scale: (1.0, 1.5, 1.0),
        offset: (1.0, 0.0, 0.0),
    };
    let rt = round_trip(&msg);
    let bytes_orig = borsh::to_vec(&msg).unwrap();
    let bytes_rt = borsh::to_vec(&rt).unwrap();
    assert_eq!(bytes_orig, bytes_rt);
}

#[test]
fn parse_version_valid() {
    assert_eq!(super::parse_version("1.2.3"), Some((1, 2, 3)));
    assert_eq!(super::parse_version("0.0.0"), Some((0, 0, 0)));
    assert_eq!(super::parse_version("10.20.30"), Some((10, 20, 30)));
}

#[test]
fn parse_version_invalid() {
    assert_eq!(super::parse_version(""), None);
    assert_eq!(super::parse_version("1.2"), None);
    assert_eq!(super::parse_version("a.b.c"), None);
}
