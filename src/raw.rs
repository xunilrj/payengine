use crate::engine::Transaction;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Amounts up to this many bytes are stored inline, so a row needs no
/// allocation. Any valid `Decimal` fits; longer text goes to the heap.
const INLINE_AMOUNT: usize = 128;

#[derive(Debug, Clone, Copy)]
pub enum TransactionType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

impl TransactionType {
    pub fn parse(field: &[u8]) -> Option<Self> {
        match field {
            b"deposit" => Some(Self::Deposit),
            b"withdrawal" => Some(Self::Withdrawal),
            b"dispute" => Some(Self::Dispute),
            b"resolve" => Some(Self::Resolve),
            b"chargeback" => Some(Self::Chargeback),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Deposit => "deposit",
            Self::Withdrawal => "withdrawal",
            Self::Dispute => "dispute",
            Self::Resolve => "resolve",
            Self::Chargeback => "chargeback",
        }
    }
}

/// The amount field as raw text, not parsed yet.
pub enum RawAmount {
    Missing,
    Inline { len: u8, bytes: [u8; INLINE_AMOUNT] },
    Heap(Box<[u8]>),
}

impl RawAmount {
    pub fn new(field: &[u8]) -> Self {
        if field.is_empty() {
            Self::Missing
        } else if field.len() <= INLINE_AMOUNT {
            let mut bytes = [0; INLINE_AMOUNT];
            bytes[..field.len()].copy_from_slice(field);
            Self::Inline {
                len: field.len() as u8,
                bytes,
            }
        } else {
            Self::Heap(field.into())
        }
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Missing => None,
            Self::Inline { len, bytes } => Some(&bytes[..usize::from(*len)]),
            Self::Heap(bytes) => Some(bytes),
        }
    }
}

pub struct RawTransaction {
    pub ty: TransactionType,
    pub client: u16,
    pub tx: u32,
    pub amount: RawAmount,
    /// Where the row is in the input, to report errors.
    pub line: u64,
    pub byte: u64,
    pub amount_column: usize,
}

/// A row rejected while turning it into a transaction.
#[derive(Debug)]
pub struct RawTransactionError {
    pub line: u64,
    pub byte: u64,
    pub field: usize,
    pub message: String,
}

impl RawTransaction {
    pub fn into_transaction(self) -> Result<Transaction, RawTransactionError> {
        let (client, tx) = (self.client, self.tx);
        let amount = self.parse_amount()?;
        Ok(match self.ty {
            TransactionType::Deposit => Transaction::Deposit {
                client,
                tx,
                amount: self.required_amount(amount)?,
            },
            TransactionType::Withdrawal => Transaction::Withdrawal {
                client,
                tx,
                amount: self.required_amount(amount)?,
            },
            TransactionType::Dispute => Transaction::Dispute { client, tx },
            TransactionType::Resolve => Transaction::Resolve { client, tx },
            TransactionType::Chargeback => Transaction::Chargeback { client, tx },
        })
    }

    fn error(&self, message: String) -> RawTransactionError {
        RawTransactionError {
            line: self.line,
            byte: self.byte,
            field: self.amount_column,
            message,
        }
    }

    fn parse_amount(&self) -> Result<Option<Decimal>, RawTransactionError> {
        let Some(bytes) = self.amount.as_bytes() else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(bytes);
        Decimal::from_str(&text)
            .or_else(|_| Decimal::from_scientific(&text))
            .map(Some)
            .map_err(|_| self.error(format!("`amount` must be a decimal number, got `{text}`")))
    }

    fn required_amount(&self, amount: Option<Decimal>) -> Result<Decimal, RawTransactionError> {
        let name = self.ty.name();
        match amount {
            None => Err(self.error(format!("{name} requires an `amount`"))),
            Some(a) if a.is_sign_negative() => {
                Err(self.error(format!("{name} `amount` must not be negative")))
            }
            Some(a) => Ok(a),
        }
    }
}
