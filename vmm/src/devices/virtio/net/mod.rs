pub mod device;
pub mod queue_handler;
pub mod simple_handler;
pub mod tap;

// Size of the `virtio_net_hdr` structure defined by the standard.
pub const VIRTIO_NET_HDR_SIZE: usize = 12;

// Prob have to find better names here, but these basically represent the order of the queues.
// If the net device has a single RX/TX pair, then the former has index 0 and the latter 1. When
// the device has multiqueue support, then RX queues have indices 2k, and TX queues 2k+1.
const RXQ_INDEX: u16 = 0;
const TXQ_INDEX: u16 = 1;
