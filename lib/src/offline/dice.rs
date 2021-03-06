use crate::*;
use bitcoin::Network;
use num_bigint::BigUint;
use std::io;
use std::str::FromStr;
use structopt::StructOpt;

/// Dice generate a bitcoin master key in bip32
#[derive(StructOpt, Debug)]
#[structopt(name = "dice")]
pub struct DiceOptions {
    /// Number of faces of the dice, only platonic solid are valid (4, 6, 8, 12, 20) or a coin (2)
    #[structopt(short, long)]
    faces: Base,

    /// Number of bits of entropy
    #[structopt(short, long, default_value = "256")]
    bits: Bits,

    /// Name of the key
    #[structopt(short, long)]
    key_name: String,

    /// Value of the die launch
    #[structopt(short, required = true)]
    launches: Vec<u32>,

    /// QR code max version to use (max size)
    #[structopt(long, default_value = "14")]
    pub qr_version: i16,
}

#[derive(Debug, Clone)]
enum Bits {
    _128,
    _192,
    _256,
}

#[derive(Debug, Clone, Copy)]
enum Base {
    _2 = 2,
    _4 = 4,
    _6 = 6,
    _8 = 8,
    _12 = 12,
    _20 = 20,
}

impl DiceOptions {
    pub fn validate(&self) -> Result<()> {
        let max: BigUint = self.bits.clone().into();
        let faces = self.faces as u32;

        let count: u32 = required_dice_launches(faces, &max);
        if self.launches.len() as u32 != count {
            let bits = &format!("{:?}", self.bits)[1..];
            return Err(format!(
                "Need {} dice launches (-l) to achieve {} bits of entropy (provided: {})",
                count,
                bits,
                self.launches.len()
            )
            .into());
        }

        if self.launches.iter().any(|n| *n == 0 || *n > faces) {
            return Err(format!("Numbers must be from 1 to {} included", faces).into());
        }

        Ok(())
    }
}

pub fn roll(datadir: &str, network: Network, opt: &DiceOptions) -> Result<MasterKeyOutput> {
    opt.validate()?;

    let master_key = calculate_key(&opt.launches, opt.faces as u32, network, &opt.key_name)?;
    let output = save_keys(datadir, network, &opt.key_name, master_key, opt.qr_version)?;

    Ok(output)
}

fn multiply_dice_launches(launches: &[u32], base: u32) -> BigUint {
    let init = BigUint::from(launches[0] - 1);
    launches.iter().skip(1).fold(init, |mut sum, i| {
        sum *= base;
        sum += i - 1u32;
        sum
    })
}

fn required_dice_launches(faces: u32, max: &BigUint) -> u32 {
    // calculating the number of dice launches needed for the bigger number lesser than n
    let mut count = 0u32;
    let mut acc = BigUint::from(1u32);
    loop {
        count += 1;
        acc *= faces;
        if acc > *max {
            return count - 1;
        }
    }
}

fn calculate_key(
    launches: &[u32],
    faces: u32,
    network: Network,
    name: &str,
) -> Result<PrivateMasterKey> {
    let acc = multiply_dice_launches(&launches, faces);

    let sec = acc.to_bytes_be();
    let mut key = PrivateMasterKey::new(network, &sec, name)?;
    let dice = Dice {
        faces,
        launches: format!("{:?}", launches),
        value: acc.to_string(),
    };
    key.dice = Some(dice);

    Ok(key)
}

impl From<Bits> for BigUint {
    fn from(bits: Bits) -> Self {
        let one = BigUint::from(1u32);
        match bits {
            Bits::_128 => one << 128,
            Bits::_192 => one << 192,
            Bits::_256 => one << 256,
        }
    }
}

impl FromStr for Bits {
    type Err = io::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "128" => Ok(Bits::_128),
            "192" => Ok(Bits::_192),
            "256" => Ok(Bits::_256),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} not in (128, 192, 256)", s),
            )),
        }
    }
}

impl FromStr for Base {
    type Err = io::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "2" => Ok(Base::_2),
            "4" => Ok(Base::_4),
            "6" => Ok(Base::_6),
            "8" => Ok(Base::_8),
            "12" => Ok(Base::_12),
            "20" => Ok(Base::_20),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} not in (2, 4, 6, 8, 12, 20)", s),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::offline::dice::*;
    use crate::PrivateMasterKey;
    use bitcoin::Network;
    use num_bigint::BigUint;
    use tempdir::TempDir;

    #[test]
    fn test_roll() {
        let temp_dir = TempDir::new("test_derive_key").unwrap();
        let temp_dir_str = format!("{}/", temp_dir.path().display());
        let launches = vec![1u32; 29];
        let mut opt = DiceOptions {
            faces: Base::_20,
            bits: Bits::_128,
            key_name: "a".to_string(),
            launches,
            qr_version: 14,
        };

        roll(&temp_dir_str, Network::Testnet, &opt).unwrap();

        opt.launches = vec![1u32; 28];
        opt.key_name = "b".to_string();
        let result = roll(&temp_dir_str, Network::Testnet, &opt);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Need 29 dice launches (-l) to achieve 128 bits of entropy (provided: 28)"
        );

        opt.launches = vec![21u32; 29];
        opt.key_name = "c".to_string();
        let result = roll(&temp_dir_str, Network::Testnet, &opt);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Numbers must be from 1 to 20 included"
        );

        let mut launches = vec![1u32; 29];
        if let Some(last) = launches.last_mut() {
            *last = 2;
        }
        opt.launches = launches;
        opt.key_name = "d".to_string();
        let master_key = roll(&temp_dir_str, Network::Bitcoin, &opt).unwrap();
        assert_eq!(master_key.key.dice.unwrap().value, "1");
        assert_eq!("xprv9s21ZrQH143K3YSbAXLMPCzJso5QAarQksAGc5rQCyZCBfw4Rj2PqVLFNgezSBhktYkiL3Ta2stLPDF9yZtLMaxk6Spiqh3DNFG8p8MVeEC", master_key.key.xprv.to_string());
    }

    #[test]
    fn test_bits() -> Result<()> {
        let bits: Bits = "128".parse()?;
        let number: BigUint = bits.into();
        assert_eq!(
            "340282366920938463463374607431768211456",
            format!("{}", number)
        );
        Ok(())
    }

    #[test]
    fn test_required_dice_launches() {
        assert_eq!(required_dice_launches(6, &BigUint::from(5u32)), 0);
        assert_eq!(required_dice_launches(6, &BigUint::from(6u32)), 1);
        assert_eq!(required_dice_launches(6, &BigUint::from(7u32)), 1);
        assert_eq!(required_dice_launches(6, &BigUint::from(35u32)), 1);
        assert_eq!(required_dice_launches(6, &BigUint::from(36u32)), 2);
        assert_eq!(required_dice_launches(6, &BigUint::from(37u32)), 2);
        assert_eq!(required_dice_launches(256, &BigUint::from(7u32)), 0);
        let n: BigUint = Bits::_256.into();
        assert_eq!(required_dice_launches(256, &n), 32);
        assert_eq!(required_dice_launches(8, &n), 85);
        assert_eq!(required_dice_launches(6, &n), 99);
        let n: BigUint = Bits::_128.into();
        assert_eq!(required_dice_launches(256, &n), 16);
        let n: BigUint = Bits::_192.into();
        assert_eq!(required_dice_launches(256, &n), 24);
    }

    #[test]
    fn test_multiply_dice_launches() {
        assert_eq!(multiply_dice_launches(&vec![6, 6], 6), BigUint::from(35u32));
        assert_eq!(multiply_dice_launches(&vec![6], 6), BigUint::from(5u32));
        assert_eq!(
            multiply_dice_launches(&vec![10, 10], 10),
            BigUint::from(99u32)
        );
        assert_eq!(
            multiply_dice_launches(&vec![1, 1, 1], 2),
            BigUint::from(0u32)
        );
        assert_eq!(multiply_dice_launches(&vec![2], 2), BigUint::from(1u32));
    }

    #[test]
    fn test_master_from_dice() {
        // priv1.key and priv2.key taken from https://github.com/tyler-smith/go-bip32/blob/master/bip32_test.go

        let bytes = include_bytes!("../../test_data/dice/priv1.key");
        let expected: PrivateMasterKey = serde_json::from_slice(bytes).unwrap();
        let calculated = calculate_key(&vec![2], 2, Network::Bitcoin, "name").unwrap();

        assert_eq!(calculated, expected);

        let bytes = include_bytes!("../../test_data/dice/priv2.key");
        let expected: PrivateMasterKey = serde_json::from_slice(bytes).unwrap();
        let calculated = calculate_key(
            &vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 1],
            256,
            Network::Bitcoin,
            "name",
        )
        .unwrap();
        assert_eq!(
            calculated.fingerprint.to_string(),
            expected.fingerprint.to_string()
        );
        assert_eq!(calculated, expected);
    }
}
