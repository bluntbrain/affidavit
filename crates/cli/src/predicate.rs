//! `--where` parsing: `log.address == 0xc02a..., log.data0 >= 1000000`.

use affidavit_core::statement::{Clause, Field, Op, Predicate};
use alloy_primitives::U256;
use anyhow::{anyhow, bail, Result};

fn field(name: &str) -> Result<Field> {
    let name = name.trim().to_ascii_lowercase();
    Ok(match name.as_str() {
        "account.balance" => Field::Balance,
        "account.nonce" => Field::Nonce,
        "account.code_hash" => Field::CodeHash,
        "storage.value" => Field::StorageValue,
        "log.address" => Field::LogAddress,
        "tx.from" => Field::TxFrom,
        "tx.to" => Field::TxTo,
        "tx.value" => Field::TxValue,
        "tx.success" => Field::TxSuccess,
        "block.number" => Field::BlockNumber,
        "block.timestamp" => Field::BlockTimestamp,
        other => {
            if let Some(n) = other.strip_prefix("log.topic") {
                Field::LogTopic(n.parse()?)
            } else if let Some(n) = other.strip_prefix("log.data") {
                Field::LogDataWord(n.parse()?)
            } else {
                bail!("unknown field `{other}`")
            }
        }
    })
}

fn value(raw: &str) -> Result<U256> {
    let raw = raw.trim().replace('_', "");
    if let Some(hex) = raw.strip_prefix("0x") {
        return Ok(U256::from_str_radix(hex, 16)?);
    }
    if let Some((mantissa, exp)) = raw.split_once('e') {
        return Ok(U256::from_str_radix(mantissa, 10)? * U256::from(10u8).pow(U256::from(exp.parse::<u64>()?)));
    }
    Ok(U256::from_str_radix(&raw, 10)?)
}

pub fn parse(spec: &str) -> Result<Predicate> {
    let mut clauses = Vec::new();
    for part in spec.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (lhs, op, rhs) = ["==", "!=", "<=", ">=", "<", ">"]
            .iter()
            .find_map(|op| part.split_once(op).map(|(l, r)| (l, *op, r)))
            .ok_or_else(|| anyhow!("no comparison in `{part}`"))?;
        let op = match op {
            "==" => Op::Eq,
            "!=" => Op::Ne,
            "<=" => Op::Le,
            ">=" => Op::Ge,
            "<" => Op::Lt,
            _ => Op::Gt,
        };
        clauses.push(Clause { field: field(lhs)?, op, value: value(rhs)? });
    }
    Ok(Predicate(clauses))
}

pub fn describe(predicate: &Predicate) -> String {
    if predicate.0.is_empty() {
        return "(none)".into();
    }
    predicate
        .0
        .iter()
        .map(|c| {
            let op = match c.op {
                Op::Eq => "==",
                Op::Ne => "!=",
                Op::Lt => "<",
                Op::Le => "<=",
                Op::Gt => ">",
                Op::Ge => ">=",
            };
            let (name, value) = match c.field {
                Field::Balance => ("account.balance".to_string(), c.value.to_string()),
                Field::Nonce => ("account.nonce".to_string(), c.value.to_string()),
                Field::CodeHash => ("account.code_hash".to_string(), format!("{:#066x}", c.value)),
                Field::StorageValue => ("storage.value".to_string(), c.value.to_string()),
                Field::LogAddress => ("log.address".to_string(), format!("{:#042x}", c.value)),
                Field::LogTopic(i) => (format!("log.topic{i}"), format!("{:#066x}", c.value)),
                Field::LogDataWord(i) => (format!("log.data{i}"), c.value.to_string()),
                Field::TxFrom => ("tx.from".to_string(), format!("{:#042x}", c.value)),
                Field::TxTo => ("tx.to".to_string(), format!("{:#042x}", c.value)),
                Field::TxValue => ("tx.value".to_string(), c.value.to_string()),
                Field::TxSuccess => ("tx.success".to_string(), c.value.to_string()),
                Field::BlockNumber => ("block.number".to_string(), c.value.to_string()),
                Field::BlockTimestamp => ("block.timestamp".to_string(), c.value.to_string()),
            };
            format!("{name} {op} {value}")
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clauses() {
        let p =
            parse("log.address == 0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2, log.topic0 != 0x01; log.data0 >= 1e18")
                .unwrap();
        assert_eq!(p.0.len(), 3);
        assert_eq!(p.0[2].value, U256::from(10u64).pow(U256::from(18u64)));
        assert_eq!(p.0[1].op, Op::Ne);
        assert!(parse("tx.value => 1").is_err());
    }
}
