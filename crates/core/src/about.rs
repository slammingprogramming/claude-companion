// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! Who makes Claude Companion, how to reach them, and how to support the project. Shown by
//! `claude-companion about` and in the app's Settings; nothing here is sent anywhere.

use serde::Serialize;

/// Where to message the developer: a SimpleX contact link (private, no account needed).
pub const CONTACT_SIMPLEX: &str = "https://smp14.simplex.im/a#3gZ-zeHs4QrFZKLAN0o3SC_XQJXhj1eYBVTO_c0FAtg";

/// Where to give money in an ordinary currency.
pub const DONATE_KOFI: &str = "https://ko-fi.com/scenprogramming";

/// Where the source and the releases are.
pub const PROJECT: &str = "https://github.com/slammingprogramming/claude-companion";

/// A cryptocurrency address to donate to.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Coin {
    pub name: &'static str,
    pub ticker: &'static str,
    pub address: &'static str,
    /// Anything to know before sending.
    pub note: &'static str,
}

pub const COINS: &[Coin] = &[
    Coin {
        name: "Bitcoin",
        ticker: "BTC",
        address: "sp1qq2637t6ve9zp4wh0e4ly2u7h7jwfn0k6c4535lxpp3wrqvmkua6xjqkwevyz0wwqx5ly2f0q29qh48wgf9gl0zt049za9kr9m8xgkey4cyggxa0x",
        note: "a Silent Payments address (sp1…): your wallet must support Silent Payments (BIP 352)",
    },
    Coin {
        name: "Ethereum",
        ticker: "ETH",
        address: "0x8F238db898102125Fd968ed7CaD959Cb594a7e93",
        note: "",
    },
    Coin {
        name: "Monero",
        ticker: "XMR",
        address: "8AYK93mQEzsFFEFkuPN7wC9rPcXCyEXa1Ye5bH9U2GR3PpaZiYc84SxjMcdSdVMaaQUGmCXYkuxdDSG5bM3EVyqR25nbumV",
        note: "",
    },
];

/// The plain text `claude-companion about` prints.
pub fn text() -> String {
    let mut out = format!(
        "Claude Companion {}\nFree software under the GNU AGPL, version 3 or later. Not affiliated with Anthropic.\n\n",
        crate::VERSION
    );
    out.push_str(&format!("Source and releases: {PROJECT}\n"));
    out.push_str(&format!("Contact (SimpleX):   {CONTACT_SIMPLEX}\n"));
    out.push_str(&format!("Donate (Ko-fi):      {DONATE_KOFI}\n\nCrypto:\n"));
    for c in COINS {
        out.push_str(&format!("  {} ({})  {}\n", c.name, c.ticker, c.address));
        if !c.note.is_empty() {
            out.push_str(&format!("      {}\n", c.note));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_links_and_addresses_have_the_shape_of_what_they_claim_to_be() {
        assert!(CONTACT_SIMPLEX.starts_with("https://") && CONTACT_SIMPLEX.contains("simplex.im/a#"));
        assert!(DONATE_KOFI.starts_with("https://ko-fi.com/"));
        let by = |t: &str| COINS.iter().find(|c| c.ticker == t).expect(t).address;
        let eth = by("ETH");
        assert!(eth.len() == 42 && eth.starts_with("0x") && eth[2..].bytes().all(|b| b.is_ascii_hexdigit()));
        let xmr = by("XMR");
        assert!(xmr.len() == 95 && xmr.starts_with('8') && xmr.bytes().all(|b| b.is_ascii_alphanumeric()), "{xmr}");
        // Bech32m (BIP 350/352): lower case, no 1, b, i or o after the separator.
        let btc = by("BTC");
        assert!(btc.starts_with("sp1q") && btc.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        assert!(btc[3..].bytes().all(|b| !b"bio1".contains(&b)), "{btc}");
        for c in COINS {
            assert!(!c.address.contains(char::is_whitespace));
        }
    }

    #[test]
    fn the_text_lists_everything_once() {
        let t = text();
        assert!(t.contains(CONTACT_SIMPLEX) && t.contains(DONATE_KOFI));
        for c in COINS {
            assert_eq!(t.matches(c.address).count(), 1);
        }
    }
}
