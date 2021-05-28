Magnetite
=========

A Rust library for building peer-to-peer connected services.

When you have multiple services that need to communicate with each other,
but can't (or don't want to) configure how many services are in the network
with which ports, then peer-to-peer networking is a good solution. There are
some good libraries that already exist in this space, but they leave a lot
of work to the application author to structure the p2p communication.


## Structure

This library intends to provide a base crate like [r2d2][r2d2] (or more accurately
[bb8][bb8], the async version). The `magnetite` crate provides a generic set of
traits for managing p2p connections and handling client and server message
production/consumption. In addition, there are 2 collections of crates provided:
for specific p2p connections ([libp2p][libp2p], Kademlia, etc.), and for specific
client/server architectures (Storage, Relays, etc.).


## Usage

To use `magnetite` in your crate, you'll need to include 4 crates in your `Cargo.toml`:

```toml
[dependencies]
# for running the app
magnetite = { git = "https://github.com/erglabs/magnetite" }
# backend for peer-to-peer connections
magnetite_libp2p = { git = "https://github.com/erglabs/magnetite" }
# framework for handling messages
magnetite_storage = { git = "https://github.com/erglabs/magnetite" }
# async runtime
tokio = { version = "1", features = ["rt-threaded", "net"] }
```


## Examples

Check out the `examples` folder to see sample usage of `magnetite`.


## License

This project is licensed under the [MPL license][mpl].


[mpl]: https://www.mozilla.org/en-US/MPL/2.0
[r2d2]: https://github.com/sfackler/r2d2
[bb8]: https://github.com/djc/bb8
[libp2p]: https://libp2p.io
