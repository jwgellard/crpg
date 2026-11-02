//! K3: the bounded two-ended link.

use super::{LinkFull, LocalLink, LINK_FRAMES};
use crpg_net::transport::Transport;

/// K3 / B5: a full direction refuses the send and changes nothing; the queue
/// is FIFO. The control is a cap of three, which accepts the third frame.
#[test]
fn a_full_direction_refuses_and_stays_fifo() {
    let link = LocalLink::new(2);
    let mut client = link.client_end();
    assert_eq!(client.send(b"a"), Ok(()));
    assert_eq!(client.send(b"b"), Ok(()));
    assert_eq!(link.queued(), (2, 0));
    assert_eq!(client.send(b"c"), Err(LinkFull));
    assert_eq!(link.queued(), (2, 0), "a refused send changes nothing");

    let mut authority = link.authority_end();
    assert_eq!(authority.recv(), Some(b"a".to_vec()));
    assert_eq!(authority.recv(), Some(b"b".to_vec()));
    assert_eq!(authority.recv(), None);

    let control = LocalLink::new(3);
    let mut control_client = control.client_end();
    assert_eq!(control_client.send(b"a"), Ok(()));
    assert_eq!(control_client.send(b"b"), Ok(()));
    assert_eq!(control_client.send(b"c"), Ok(()));
    assert_eq!(control.queued(), (3, 0));
}

/// K3 / B5: the two directions are independent FIFOs.
#[test]
fn each_direction_has_its_own_queue() {
    let link = LocalLink::new(LINK_FRAMES);
    let mut authority = link.authority_end();
    let mut client = link.client_end();
    assert_eq!(authority.send(b"down"), Ok(()));
    assert_eq!(client.send(b"up"), Ok(()));
    assert_eq!(link.queued(), (1, 1));
    assert_eq!(client.recv(), Some(b"down".to_vec()));
    assert_eq!(authority.recv(), Some(b"up".to_vec()));
    assert_eq!(client.recv(), None);
    assert_eq!(authority.recv(), None);
    assert_eq!(link.queued(), (0, 0));
    assert_eq!(LINK_FRAMES, 128);
}
