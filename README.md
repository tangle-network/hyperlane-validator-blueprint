# <h1 align="center"> Hyperlane Validator Blueprint 🌐 </h1>

## 📚 Overview

This blueprint contains tasks for an operator to initialize and manage their
own [Hyperlane validator](https://docs.hyperlane.xyz/docs/operate/overview-agents#validators).

## 🚀 Features

This Blueprint provides the following key feature:

* Automated devops for running Hyperlane validators
* Tangle Network integration for on-demand instancing of validators

## 📋 Pre-requisites

* [Docker](https://docs.docker.com/engine/install/)
* [cargo-tangle](https://crates.io/crates/cargo-tangle)

## 💻 Usage

To use this blueprint:

1. Review the blueprint specifications in the `src/` directory.
2. Follow the [Hyperlane documentation](https://docs.hyperlane.xyz/docs/operate/validators/run-validators) to understand the
   validator setup process.
3. Adapt the blueprint to your specific validator configuration needs.
4. Deploy the blueprint on the Tangle Network using the Tangle CLI:

```shell
$ cargo tangle blueprint deploy
```

Upon deployment, the Blueprint will be able to be instanced and executed by any Tangle operator registered on the
blueprint.

### Starting a validator

There are two ways to start a validator:

1. With user-generated configs, and optional origin chain
2. With the [default configs](https://github.com/hyperlane-xyz/hyperlane-monorepo/tree/main/rust/main/config), and
   specified origin chain

Once you've determined which path to choose, you can call the `set_config` job.

#### Set config job

To spin up a validator instance, use the `set_config` job:

This job will save the existing config, attempt to start the validator with the new config(s), and on failure will spin back
up using the old config.

It has two parameters:

1. `config_urls`: Optional config file URLs, if not specified it will use
   the [defaults](https://github.com/hyperlane-xyz/hyperlane-monorepo/tree/main/rust/main/config).
2. `origin_chain_name`: The name of the chain being validated

**NOTE: Ensure that when using a manually specified config, `originChainName` is specified, either as a job parameter or in
the config itself**

## 🔗 External Links

- [Hyperlane Documentation](https://docs.hyperlane.xyz)
- [Tangle Network](https://www.tangle.tools/)

## 📜 License

Licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## 📬 Feedback and Contributions

We welcome feedback and contributions to improve this blueprint.
Please open an issue or submit a pull request on our GitHub repository.
Please let us know if you fork this blueprint and extend it too!

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.