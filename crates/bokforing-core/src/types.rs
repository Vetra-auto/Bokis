use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Sub, Neg};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Money(i64);

impl Money {
    pub fn from_ore(ore: i64) -> Self {
        Self(ore)
    }

    pub fn from_kr(kronor: i64, ore: i64) -> Self {
        Self(kronor * 100 + ore)
    }

    pub fn ore(&self) -> i64 {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn is_positive(&self) -> bool {
        self.0 > 0
    }

    pub fn is_negative(&self) -> bool {
        self.0 < 0
    }

    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }
}

impl Add for Money {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Money {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Money {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kronor = self.0 / 100;
        let ore = (self.0 % 100).abs();
        if kronor < 0 && ore > 0 {
            write!(f, "-{},{:02}", kronor.abs(), ore)
        } else {
            write!(f, "{},{:02}", kronor, ore)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AccountNumber(u16);

impl AccountNumber {
    pub fn new(number: u16) -> Result<Self, &'static str> {
        if !(1000..=9999).contains(&number) {
            return Err("Kontonummer måste vara mellan 1000 och 9999");
        }
        Ok(Self(number))
    }

    pub fn value(&self) -> u16 {
        self.0
    }

    pub fn account_class(&self) -> u8 {
        (self.0 / 1000) as u8
    }
}

impl fmt::Display for AccountNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
}

impl AccountType {
    pub fn from_account_number(num: &AccountNumber) -> Self {
        match num.account_class() {
            1 => AccountType::Asset,
            2 => AccountType::Liability,
            3 => AccountType::Revenue,
            4..=7 => AccountType::Expense,
            8 => AccountType::Revenue,
            9 => AccountType::Expense,
            _ => AccountType::Asset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoucherStatus {
    Draft,
    Posted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeriodStatus {
    Open,
    UnderReview,
    Closed,
    Locked,
}

impl PeriodStatus {
    pub fn allows_posting(&self) -> bool {
        matches!(self, PeriodStatus::Open)
    }

    pub fn is_sealed(&self) -> bool {
        matches!(self, PeriodStatus::Closed | PeriodStatus::Locked)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    Bokforare,
    Attest,
    Revisor,
}

impl Role {
    pub fn can_create_voucher(&self) -> bool {
        matches!(self, Role::Bokforare | Role::Admin)
    }

    pub fn can_post_voucher(&self) -> bool {
        matches!(self, Role::Bokforare | Role::Admin)
    }

    pub fn can_correct_voucher(&self) -> bool {
        matches!(self, Role::Bokforare | Role::Admin)
    }

    pub fn can_close_period(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_lock_period(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_reopen_period(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_export(&self) -> bool {
        true
    }

    pub fn can_manage_users(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_manage_accounts(&self) -> bool {
        matches!(self, Role::Admin | Role::Bokforare)
    }

    pub fn can_view_audit_log(&self) -> bool {
        matches!(self, Role::Admin | Role::Revisor)
    }

    pub fn can_manage_backups(&self) -> bool {
        matches!(self, Role::Admin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentType {
    ReceivedOriginal,
    InternalVoucher,
    SharedVoucher,
    ReferenceVoucher,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_arithmetic() {
        let a = Money::from_ore(10050);
        let b = Money::from_ore(5025);
        assert_eq!((a + b).ore(), 15075);
        assert_eq!((a - b).ore(), 5025);
        assert_eq!((-a).ore(), -10050);
    }

    #[test]
    fn money_display() {
        assert_eq!(Money::from_ore(10050).to_string(), "100,50");
        assert_eq!(Money::from_ore(0).to_string(), "0,00");
        assert_eq!(Money::from_ore(-5075).to_string(), "-50,75");
        assert_eq!(Money::from_kr(1000, 0).to_string(), "1000,00");
    }

    #[test]
    fn account_number_validation() {
        assert!(AccountNumber::new(1000).is_ok());
        assert!(AccountNumber::new(9999).is_ok());
        assert!(AccountNumber::new(999).is_err());
        assert!(AccountNumber::new(10000).is_err());
    }

    #[test]
    fn account_class_from_number() {
        let acc = AccountNumber::new(1910).unwrap();
        assert_eq!(acc.account_class(), 1);
        assert_eq!(AccountType::from_account_number(&acc), AccountType::Asset);

        let acc = AccountNumber::new(2440).unwrap();
        assert_eq!(AccountType::from_account_number(&acc), AccountType::Liability);

        let acc = AccountNumber::new(3010).unwrap();
        assert_eq!(AccountType::from_account_number(&acc), AccountType::Revenue);

        let acc = AccountNumber::new(5010).unwrap();
        assert_eq!(AccountType::from_account_number(&acc), AccountType::Expense);
    }

    #[test]
    fn period_status_semantics() {
        assert!(PeriodStatus::Open.allows_posting());
        assert!(!PeriodStatus::UnderReview.allows_posting());
        assert!(!PeriodStatus::Closed.allows_posting());
        assert!(!PeriodStatus::Locked.allows_posting());
        assert!(PeriodStatus::Closed.is_sealed());
        assert!(PeriodStatus::Locked.is_sealed());
    }

    #[test]
    fn role_capabilities() {
        assert!(Role::Bokforare.can_create_voucher());
        assert!(Role::Admin.can_create_voucher());
        assert!(!Role::Attest.can_create_voucher());
        assert!(!Role::Revisor.can_create_voucher());
        assert!(Role::Revisor.can_view_audit_log());
        assert!(Role::Revisor.can_export());
        assert!(!Role::Revisor.can_manage_users());
    }
}
