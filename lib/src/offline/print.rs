use crate::list::ListOptions;
use crate::*;
use bitcoin::consensus::serialize;
use bitcoin::util::bip32::{DerivationPath, Fingerprint};
use bitcoin::util::key;
use bitcoin::{Address, Amount, Network, OutPoint, Script, SignedAmount, TxOut};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use structopt::StructOpt;

type HDKeypaths = BTreeMap<key::PublicKey, (Fingerprint, DerivationPath)>;

/// Sign a Partially Signed Bitcoin Transaction (PSBT) with a key.
#[derive(StructOpt, Debug, Serialize, Deserialize)]
#[structopt(name = "firma")]
pub struct PrintOptions {
    /// PSBT json file
    psbt_file: PathBuf,
}

pub fn start(datadir: &str, network: Network, opt: &PrintOptions) -> Result<PsbtPrettyPrint> {
    let psbt = read_psbt(&opt.psbt_file)?;
    let kind = Kind::Wallet;
    let opt = ListOptions { kind };
    let result = common::list::list(datadir, network, &opt)?;
    let wallets: Vec<WalletJson> = result.wallets.iter().map(|w| w.wallet.clone()).collect();
    let output = pretty_print(&psbt, network, &wallets)?;
    Ok(output)
}

pub fn pretty_print(
    psbt: &PSBT,
    network: Network,
    wallets: &[WalletJson],
) -> Result<PsbtPrettyPrint> {
    let mut result = PsbtPrettyPrint::default();
    let mut previous_outputs: Vec<TxOut> = vec![];
    let mut output_values: Vec<u64> = vec![];
    let tx = &psbt.global.unsigned_tx;
    let vouts: Vec<OutPoint> = tx.input.iter().map(|el| el.previous_output).collect();
    for (i, input) in psbt.inputs.iter().enumerate() {
        let previous_output = match (&input.non_witness_utxo, &input.witness_utxo) {
            (Some(prev_tx), None) => {
                let outpoint = *vouts.get(i).ok_or_else(fn_err("can't find outpoint"))?;
                assert_eq!(prev_tx.txid(), outpoint.txid);
                prev_tx
                    .output
                    .get(outpoint.vout as usize)
                    .ok_or_else(fn_err("can't find txout"))?
            }
            (None, Some(val)) => val,
            _ => return Err("witness_utxo and non_witness_utxo are both None or both Some".into()),
        };
        previous_outputs.push(previous_output.clone());
    }
    let input_values: Vec<u64> = previous_outputs.iter().map(|o| o.value).collect();
    let mut balances = HashMap::new();

    for (i, input) in tx.input.iter().enumerate() {
        let keypaths = &psbt.inputs[i].hd_keypaths;
        let wallets = which_wallet(keypaths, &wallets);
        let txin = TxInOut {
            outpoint: Some(input.previous_output.to_string()),
            address: None,
            value: Amount::from_sat(previous_outputs[i].value).to_string(),
            path: derivation_paths(keypaths),
            wallet: wallets.join(", "),
        };
        for wallet in wallets {
            *balances.entry(wallet).or_insert(0i64) -= previous_outputs[i].value as i64
        }
        result.inputs.push(txin);
    }

    for (i, output) in tx.output.iter().enumerate() {
        let addr = Address::from_script(&output.script_pubkey, network)
            .ok_or_else(fn_err("non default script"))?;
        let keypaths = &psbt.outputs[i].hd_keypaths;
        let wallets = which_wallet(keypaths, &wallets);
        let txout = TxInOut {
            outpoint: None,
            address: Some(addr.to_string()),
            value: Amount::from_sat(output.value).to_string(),
            path: derivation_paths(keypaths),
            wallet: wallets.join(" ,"),
        };
        for wallet in wallets {
            *balances.entry(wallet).or_insert(0i64) += output.value as i64
        }
        result.outputs.push(txout);
        output_values.push(output.value);
    }
    let balances_vec: Vec<String> = balances
        .iter()
        .map(|(k, v)| format!("{}: {}", k, SignedAmount::from_sat(*v).to_string()))
        .collect();
    result.balances = balances_vec.join("\n");

    // Privacy analysis
    // Detect different script types in the outputs
    let mut script_types = HashSet::new();
    for o in tx.output.iter() {
        script_types.insert(script_type(&o.script_pubkey));
    }
    if script_types.len() > 1 {
        result.info.push("Privacy: outputs have different script types https://en.bitcoin.it/wiki/Privacy#Sending_to_a_different_script_type".to_string());
    }

    // Detect rounded amounts
    let divs: Vec<u8> = tx
        .output
        .iter()
        .map(|o| biggest_dividing_pow(o.value))
        .collect();
    if let (Some(max), Some(min)) = (divs.iter().max(), divs.iter().min()) {
        if max - min >= 3 {
            result.info.push("Privacy: outputs have different precision https://en.bitcoin.it/wiki/Privacy#Round_numbers".to_string());
        }
    }

    // Detect unnecessary input heuristic
    if previous_outputs.len() > 1 {
        if let Some(smallest_input) = input_values.iter().min() {
            if output_values.iter().any(|value| value < smallest_input) {
                result.info.push("Privacy: smallest output is smaller then smallest input https://en.bitcoin.it/wiki/Privacy#Unnecessary_input_heuristic".to_string());
            }
        }
    }

    // Detect script reuse
    let input_scripts: HashSet<Script> = previous_outputs
        .iter()
        .map(|o| o.script_pubkey.clone())
        .collect();
    if tx
        .output
        .iter()
        .any(|o| input_scripts.contains(&o.script_pubkey))
    {
        result.info.push(
            "Privacy: address reuse https://en.bitcoin.it/wiki/Privacy#Address_reuse".to_string(),
        );
    }

    let fee = input_values.iter().sum::<u64>() - output_values.iter().sum::<u64>();
    let tx_vbytes = tx.get_weight() / 4;
    let estimated_tx_vbytes = estimate_weight(psbt)? / 4;
    let estimated_fee_rate = fee as f64 / estimated_tx_vbytes as f64;

    result.size = Size {
        estimated: estimated_tx_vbytes,
        unsigned: tx_vbytes,
        psbt: serialize(psbt).len(),
    };
    result.fee = Fee {
        absolute: fee,
        absolute_fmt: Amount::from_sat(fee).to_string(),
        rate: estimated_fee_rate,
    };

    Ok(result)
}

fn biggest_dividing_pow(num: u64) -> u8 {
    let mut start = 10u64;
    let mut count = 0u8;
    loop {
        if num % start != 0 {
            return count;
        }
        start *= 10;
        count += 1;
    }
}

const SCRIPT_TYPE_FN: [fn(&Script) -> bool; 5] = [
    Script::is_p2pk,
    Script::is_p2pkh,
    Script::is_p2sh,
    Script::is_v0_p2wpkh,
    Script::is_v0_p2wsh,
];
fn script_type(script: &Script) -> Option<usize> {
    SCRIPT_TYPE_FN.iter().position(|f| f(script))
}

pub fn derivation_paths(hd_keypaths: &HDKeypaths) -> String {
    let mut vec: Vec<String> = hd_keypaths
        .iter()
        .map(|(_, (_, p))| format!("{:?}", p))
        .collect();
    vec.sort();
    vec.dedup();
    vec.join(", ")
}

fn which_wallet(hd_keypaths: &HDKeypaths, wallets: &[WalletJson]) -> Vec<String> {
    // TODO this should be done with miniscript
    let mut result = vec![];
    for wallet in wallets {
        if !hd_keypaths.is_empty()
            && hd_keypaths
                .iter()
                .all(|(_, (f, _))| wallet.fingerprints.contains(f))
        {
            result.push(wallet.name.to_string())
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::offline::print::{biggest_dividing_pow, script_type};

    #[test]
    fn test_biggest_dividing_pow() {
        assert_eq!(biggest_dividing_pow(3), 0);
        assert_eq!(biggest_dividing_pow(10), 1);
        assert_eq!(biggest_dividing_pow(11), 0);
        assert_eq!(biggest_dividing_pow(110), 1);
        assert_eq!(biggest_dividing_pow(1100), 2);
        assert_eq!(biggest_dividing_pow(1100030), 1);
    }

    #[test]
    fn test_script_type() {
        macro_rules! hex_script (($s:expr) => (bitcoin::blockdata::script::Script::from(::hex::decode($s).unwrap())));

        let s =
            hex_script!("21021aeaf2f8638a129a3156fbe7e5ef635226b0bafd495ff03afe2c843d7e3a4b51ac");
        assert_eq!(script_type(&s), Some(0usize));

        let s = hex_script!("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac");
        assert_eq!(script_type(&s), Some(1usize));

        let s = hex_script!("a914acc91e6fef5c7f24e5c8b3f11a664aa8f1352ffd87");
        assert_eq!(script_type(&s), Some(2usize));

        let s = hex_script!("00140c3e2a4e0911aac188fe1cba6ef3d808326e6d0a");
        assert_eq!(script_type(&s), Some(3usize));

        let s = hex_script!("00201775ead41acefa14d2d534d6272da610cc35855d0de4cab0f5c1a3f894921989");
        assert_eq!(script_type(&s), Some(4usize));
    }
}
