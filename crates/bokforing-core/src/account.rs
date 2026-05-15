use crate::types::AccountNumber;

pub fn validate_account_number(number: u16) -> Result<AccountNumber, &'static str> {
    AccountNumber::new(number)
}

pub fn is_balance_account(num: &AccountNumber) -> bool {
    num.account_class() <= 2
}

pub fn is_result_account(num: &AccountNumber) -> bool {
    num.account_class() >= 3
}
