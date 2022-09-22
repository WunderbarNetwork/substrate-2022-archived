# This is the build stage for Polkadot. Here we create the binary in a temporary image.
FROM docker.io/paritytech/ci-linux:production as builder

WORKDIR /substrate
COPY . /substrate

RUN rustup show
RUN cargo build --locked --release
RUN /substrate/target/release/substrate --version

# This is the 2nd stage: a very small image where we copy the Polkadot binary."
FROM docker.io/library/ubuntu:20.04

LABEL description="Multistage Docker image for substrate, a platform for web3" \
  io.parity.image.type="builder" \
  io.parity.image.authors="dan@wunderbar.network" \
  io.parity.image.vendor="wunberbar network" \
  io.parity.image.description="substrate: a platform for web3" \
  io.parity.image.documentation="https://github.com/paritytech/polkadot/"

COPY --from=builder /substrate/target/release/substrate /usr/local/bin

RUN useradd -m -u 1000 -U -s /bin/sh -d /substrate-runner substrate-runner
RUN mkdir -p /data /substrate/.local/share /chain-data
RUN chown -R substrate-runner:substrate-runner /data /chain-data
RUN ln -s /data /substrate/.local/share/substrate-runner

# check if executable works in this container
RUN /usr/local/bin/substrate --version

# unclutter and minimize the attack surface
RUN  rm -rf /usr/bin /usr/sbin

USER substrate-runner

EXPOSE 30333 9933 9944 9615
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/substrate"]