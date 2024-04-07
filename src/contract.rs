use cosmwasm_std::{
    entry_point, to_binary, Addr, BankMsg, Binary, Coin, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, Loan, LOANS};
use cw2::set_contract_version;
use cw20::Cw20ExecuteMsg;

// Version info, for migration info
const CONTRACT_NAME: &str = "RWA Lending and Borrowing Contract";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let config = Config {
        arbiter: deps.api.addr_validate(&msg.arbiter)?,
        recipient: deps.api.addr_validate(&msg.recipient)?,
        source: info.sender,
        expiration: msg.expiration,
    };

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    if let Some(expiration) = msg.expiration {
        if expiration.is_expired(&env.block) {
            return Err(ContractError::Expired { expiration });
        }
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::LendToken {
            asset_address,
            duration,
            collateral_amount,
            daily_fee_amount,
            max_rent_days,
            ipfs_cid,
        } => execute_lend_token(
            deps,
            env,
            info,
            asset_address,
            duration,
            collateral_amount,
            daily_fee_amount,
            max_rent_days,
            ipfs_cid,
        ),
        ExecuteMsg::BorrowToken { asset_address } => {
            execute_borrow_token(deps, env, info, asset_address)
        }
        ExecuteMsg::ReturnToken { asset_address } => {
            execute_return_token(deps, env, info, asset_address)
        }
        ExecuteMsg::WithdrawCollateral { amount } => {
            execute_withdraw_collateral(deps, env, info, amount)
        }
    }
}

fn execute_lend_token(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset_address: Addr,
    duration: u64,
    collateral_amount: Vec<Coin>,
    daily_fee_amount: Vec<Coin>,
    max_rent_days: u64,
    ipfs_cid: String,
) -> Result<Response, ContractError> {
    // Retrieve the toke from contract
    let loan: Loan = match LOANS.may_load(deps.storage, &asset_address)? {
        Some(loan) => loan,
        None => {
            return Err(ContractError::LoanNotFound {});
        }
    };

    // Ensure that the lender is the caller of this function
    if loan.lender != info.sender {
        return Err(ContractError::Unauthorized {});
    }

    // Ensure that the token is not already lended
    if loan.borrower != Addr::unchecked("") {
        return Err(ContractError::LoanAlreadyActive {});
    }

    // Ensure that the duration is valid
    if duration == 0 {
        return Err(ContractError::InvalidDuration {});
    }

    // Ensure that interest and amounts are provided
    if collateral_amount.is_empty() || daily_fee_amount.is_empty() {
        return Err(ContractError::InvalidAmounts {});
    }

    // Ensure that the maximum rent days is greater than zero
    if max_rent_days == 0 {
        return Err(ContractError::InvalidMaxRentDays {});
    }

    // Update the loan with new parameters
    let updated_loan = Loan {
        lender: loan.lender,
        borrower: loan.borrower,
        asset_address: loan.asset_address,
        duration,
        collateral_amount: collateral_amount.clone(),
        daily_fee_amount: daily_fee_amount.clone(),
        max_rent_days,
        ipfs_cid,
        start_time: loan.start_time,
    };

    // Save the updated loan to state
    LOANS.save(deps.storage, &asset_address, &updated_loan)?;

    // Transfer the asset to the contract
    // Return a response indicating success
    Ok(Response::default())
}

fn execute_borrow_token(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset_address: Addr,
) -> Result<Response, ContractError> {
    // Retrieve the token from contract
    let loan = LOANS.load(deps.storage, &asset_address)?;

    // Ensure the token is lended
    if !loan.is_active {
        return Err(ContractError::InvalidLoan {
            reason: "Loan is not active".to_string(),
        });
    }

    // Ensure the borrower is not the lender
    if info.sender == loan.lender {
        return Err(ContractError::Unauthorized {});
    }

    // Ensure the borrower is not already borrowing the asset
    if loan.borrower != Addr::unchecked("") {
        return Err(ContractError::InvalidLoan {
            reason: "Asset is already borrowed".to_string(),
        });
    }

    // Transfer collateral tokens from the borrower to the lender
    let msg = Cw20ExecuteMsg::Transfer {
        recipient: loan.lender.to_string(),
        amount: loan.collateral_amount,
    };
    let execute_msg = WasmMsg::Execute {
        contract_addr: loan.asset_address.to_string(),
        msg: to_binary(&msg)?,
        funds: vec![],
    };

    // Create response
    let mut response = Response::new().add_messages(vec![execute_msg.into()]);

    // Update loan with borrower information
    let updated_loan = Loan {
        borrower: info.sender.clone(),
        ..loan
    };
    LOANS.save(deps.storage, &asset_address, &updated_loan)?;

    // Return response
    Ok(response)
}

fn execute_return_token(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset_address: Addr,
) -> Result<Response, ContractError> {
    // Retrieve the token from contract
    let loan = LOANS.load(deps.storage, &asset_address)?;

    // Ensure the loan is active
    if !loan.is_active {
        return Err(ContractError::LoanNotFound {});
    }

    // Ensure the caller is the borrower
    if info.sender != loan.borrower {
        return Err(ContractError::Unauthorized {});
    }

    // Check if the loan duration has exceeded
    if env.block.time > loan.start_time + loan.duration {
        return Err(ContractError::LoanNotFound {});
    }

    // Transfer the asset back to the lender with interst amount

    // Update the loan status to inactive
    let updated_loan = Loan {
        is_active: false,
        ..loan
    };
    LOANS.save(deps.storage, &asset_address, &updated_loan)?;

    // Return a response indicating success
    Ok(Response::default())
}

fn execute_withdraw_collateral(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    amount: Vec<Coin>,
) -> Result<Response, ContractError> {
    // Load the lender's address
    let config = CONFIG.load(deps.storage)?;

    // Ensure that the sender is the lender
    if info.sender != config.source {
        return Err(ContractError::Unauthorized {});
    }

    // Transfer the specified amount of collateral to the lender
    let recipient = info.sender;
    let response = Response::new()
        .add_message(BankMsg::Send {
            to_address: recipient.into(),
            amount,
        })
        .add_attribute("action", "withdraw_collateral")
        .add_attribute("recipient", recipient.to_string());

    Ok(response)
}
