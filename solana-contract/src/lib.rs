use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke;
use anchor_lang::solana_program::system_instruction;

declare_id!("5GQn3NDZgciJmNB85Az5V5FrAY64QddUWW9kKsRDLn2a");

const PREFIX_STATE: &str= "state_8";
const PREFIX_CONFIG: &str = "config_8";
const PREFIX_BALANCE: &str = "balance_8";
const PREFIX_ORDER: &str = "order_8";
const PREFIX_RECEIPT: &str = "receipt_8";
const PREFIX_CCFEE: &str = "ccfee_8";
const ADMIN: &str = "BuTuA7YKzx5CUn3bALZcK97jQrFM94QfsBUaUdM6BCxm";

#[program]
pub mod solana_lending {
    use super::*;

    #[access_control(only_admin(&ctx.accounts.payer))]
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let global = &mut ctx.accounts.global_state;
        global.curr_order_sn = 1;
        global.curr_receipt_sn = 1;
        Ok(())
    }

    #[access_control(only_admin(&ctx.accounts.payer))]
    pub fn set_config(
        ctx: Context<SetConfig>,
        min_ir: u64,
        max_ir: u64,
        penalty_ir: u64,
        penalty_days: u64,
        commission_rate: u64,
        cycle: u64,
        deadline: u64
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.min_ir = min_ir;
        config.max_ir = max_ir;
        config.penalty_ir = penalty_ir;
        config.penalty_days = penalty_days;
        config.commission_rate = commission_rate;
        config.cycle = cycle;
        config.deadline = deadline;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        let user_balance = &mut ctx.accounts.user_balance;
        user_balance.amount += amount;
        invoke(
            &system_instruction::transfer(ctx.accounts.payer.key, ctx.accounts.user_balance.to_account_info().key, amount),
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.user_balance.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        Ok(())
    } 

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let user_balance = &mut ctx.accounts.user_balance;
        require!(user_balance.amount >= amount, MyError::InsufficientUserBalance);
        user_balance.amount -= amount;
        **ctx.accounts.user_balance.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.payer.to_account_info().try_borrow_mut_lamports()? += amount;
        Ok(())
    }

    pub fn place_order(ctx: Context<PlaceOrder>, amount: u64, rate: u64) -> Result<()> {
        let order = &mut ctx.accounts.order;
        let user_balance = &mut ctx.accounts.user_balance;
        let global = &mut ctx.accounts.global;
        let config = &ctx.accounts.config;
    
        require!(user_balance.amount >= amount, MyError::InsufficientUserBalance);
        require!(rate >= config.min_ir && rate <= config.max_ir, MyError::IllegalInterestRate);
        order.sn = global.curr_order_sn;
        order.lender = ctx.accounts.payer.key();
        order.balance = amount;
        order.rate = rate;
        user_balance.amount -= amount;
        global.curr_order_sn += 1;
        
        emit!(EventPlaceOrder {
            order_sn: order.sn,
            lender: ctx.accounts.payer.key(),
            balance: amount,
            rate: rate,
        });

        **ctx.accounts.user_balance.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.order.to_account_info().try_borrow_mut_lamports()? += amount;  
        Ok(())
    }

    pub fn cancel_order(ctx: Context<CancelOrder>, order_sn: u64) -> Result<()> {
        let order = &ctx.accounts.order;
        let user_balance = &mut ctx.accounts.user_balance;
        require!(order.lender == *ctx.accounts.payer.key, MyError::NoOrderFound);
        user_balance.amount += order.balance;
        
        emit!(EventCancelOrder {
            order_sn: order_sn,
            lender: ctx.accounts.payer.key(),
            balance: order.balance,
        });

        **ctx.accounts.order.to_account_info().try_borrow_mut_lamports()? -= order.balance;
        **ctx.accounts.user_balance.to_account_info().try_borrow_mut_lamports()? += order.balance;
        Ok(())
    }

    pub fn close_order(ctx: Context<CloseOrder>, order_sn: u64) -> Result<()> {
        let order = &ctx.accounts.order;
        require!(order.balance == 0, MyError::NonZeroOrderBalance);
        emit!(EventCloseOrder {
            order_sn: order_sn,
            lender: order.lender,
        });
        Ok(())
    }

    #[access_control(only_admin(&ctx.accounts.payer))]
    pub fn borrow(ctx: Context<Borrow>, order_sn: u64, borrower: Pubkey, amount: u64, chainid: u32, c_sn: u64, source: [u8; 20], token: [u8; 20], frozen: u64) -> Result<()> {
        let receipt = &mut ctx.accounts.receipt;
        let order = &mut ctx.accounts.order;
        let global = &mut ctx.accounts.global;
        require!(order.balance >= amount, MyError::InsufficientOrderBalance);
        
        receipt.sn = global.curr_receipt_sn;
        receipt.borrower = borrower;
        receipt.lender = order.lender;
        receipt.source = source;
        receipt.chainid = chainid;
        receipt.c_sn = c_sn;
        receipt.token = token;
        receipt.frozen = frozen;
        receipt.amount = amount;
        receipt.time = ctx.accounts.clock.unix_timestamp as u64;
        receipt.rate = order.rate;
        global.curr_receipt_sn += 1;
        order.balance -= amount;

        emit!(EventBorrowSuccess {
            receipt_sn: receipt.sn,
            borrower: borrower,
            lender: receipt.lender,
            source: source,
            chainid: chainid,
            c_sn: c_sn,
            token: token,
            frozen: frozen,
            amount: amount,
            time: receipt.time,
            rate: receipt.rate,
            order_sn: order_sn,
            order_balance: order.balance,
        });

        **ctx.accounts.order.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.recipient.try_borrow_mut_lamports()? += amount;
        Ok(())
    }

    pub fn repay(ctx: Context<Repay>, receipt_sn: u64) -> Result<()> {
        let receipt = &mut ctx.accounts.receipt;
        let config = &ctx.accounts.config;
        let ccfee = &ctx.accounts.cc_fee;
        let lender_balance = &mut ctx.accounts.lender_balance;
        require!(receipt.borrower == *ctx.accounts.payer.key, MyError::NoReceiptFound);
        
        let mut amount = receipt.amount + (receipt.amount * receipt.rate as u64) / 10000;
        let now = ctx.accounts.clock.unix_timestamp as u64;
        if now > receipt.time + config.cycle {
            let mut overdue_days = (now - receipt.time - config.cycle) / 86400 + 1;
            if overdue_days > config.penalty_days {
                overdue_days = config.penalty_days;
            }
            amount += (receipt.amount * config.penalty_ir) / 1000 * overdue_days;
        }
        let commission = (amount - receipt.amount) * config.commission_rate / 100;
        lender_balance.amount += amount - commission;
        
        invoke(
            &system_instruction::transfer(ctx.accounts.payer.key, ctx.accounts.lender_balance.to_account_info().key, amount + ccfee.fee),
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.lender_balance.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        **ctx.accounts.lender_balance.to_account_info().try_borrow_mut_lamports()? -= commission + ccfee.fee;
        **ctx.accounts.admin.try_borrow_mut_lamports()? += commission + ccfee.fee;
        
        emit!(EventRepaySuccess {
            receipt_sn: receipt_sn,
            borrower: receipt.borrower,
            lender: receipt.lender,
            amount: amount,
            income: amount - commission,
            chainid: receipt.chainid,
            c_sn: receipt.c_sn,
            source: receipt.source,
            token: receipt.token,
            frozen: receipt.frozen,
        });
        Ok(())
    }

    pub fn liquidate(ctx: Context<Liquidate>, receipt_sn: u64, receiver: [u8; 20]) -> Result<()> {
        let receipt = &mut ctx.accounts.receipt;
        let config = &ctx.accounts.config;
        let ccfee = &ctx.accounts.cc_fee;   
        require!(receipt.lender == *ctx.accounts.payer.key, MyError::NoReceiptFound);
        require!(ctx.accounts.clock.unix_timestamp as u64 > receipt.time + config.deadline, MyError::DeadlineNotMeet);

        invoke(
            &system_instruction::transfer(ctx.accounts.payer.key, ctx.accounts.admin.to_account_info().key, ccfee.fee),
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.admin.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        emit!(EventLiquidate {
            receipt_sn: receipt_sn,
            borrower: receipt.borrower,
            lender: receipt.lender,
            chainid: receipt.chainid,
            c_sn: receipt.c_sn,
            source: receipt.source,
            frozen: receipt.frozen,
            receiver: receiver,
        });
        Ok(())
    }

    #[access_control(only_admin(&ctx.accounts.payer))]
    pub fn set_ccfee(ctx: Context<SetCCFee>, _chainid: u32, fee: u64) -> Result<()> {
        let ccfee = &mut ctx.accounts.cc_fee;
        ccfee.fee = fee;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(init_if_needed, payer = payer, space = 8 + 8, seeds = [PREFIX_BALANCE.as_bytes(), payer.key().as_ref()], bump)]
    pub user_balance: Account<'info, UserBalance>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut, seeds = [PREFIX_BALANCE.as_bytes(), payer.key().as_ref()], bump)]
    pub user_balance: Account<'info, UserBalance>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PlaceOrder<'info> {
    #[account(init, payer = payer, space = 8 + 56, seeds = [PREFIX_ORDER.as_bytes(), global.curr_order_sn.to_le_bytes().as_ref()], bump)]
    pub order: Account<'info, Order>,
    #[account(mut, seeds = [PREFIX_BALANCE.as_bytes(), payer.key().as_ref()], bump)]
    pub user_balance: Account<'info, UserBalance>,
    #[account(mut, seeds = [PREFIX_STATE.as_bytes()], bump)]
    pub global: Account<'info, GlobalState>,
    #[account(seeds = [PREFIX_CONFIG.as_bytes()], bump)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(order_sn: u64)]
pub struct CancelOrder<'info> {
    #[account(mut, close = payer, seeds = [PREFIX_ORDER.as_bytes(), order_sn.to_le_bytes().as_ref()], bump)]
    pub order: Account<'info, Order>,
    #[account(mut, seeds = [PREFIX_BALANCE.as_bytes(), payer.key().as_ref()], bump)]
    pub user_balance: Account<'info, UserBalance>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(order_sn: u64)]
pub struct CloseOrder<'info> {
    #[account(mut, close = lender, seeds = [PREFIX_ORDER.as_bytes(), order_sn.to_le_bytes().as_ref()], bump)]
    pub order: Account<'info, Order>,
    /// CHECK:
    #[account(mut, constraint = *lender.key == order.lender)]
    pub lender: AccountInfo<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(order_sn: u64, borrower: Pubkey)]
pub struct Borrow<'info> {
    #[account(init, payer = payer, space = 8 + 156, seeds = [PREFIX_RECEIPT.as_bytes(), global.curr_receipt_sn.to_le_bytes().as_ref()], bump)]
    pub receipt: Account<'info, LoanReceipt>,
    #[account(mut, seeds = [PREFIX_ORDER.as_bytes(), order_sn.to_le_bytes().as_ref()], bump)]
    pub order: Account<'info, Order>,
    #[account(mut, seeds = [PREFIX_STATE.as_bytes()], bump)]
    pub global: Account<'info, GlobalState>,
    /// CHECK: 
    #[account(mut, constraint = *recipient.key == borrower)]
    pub recipient: AccountInfo<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(receipt_sn: u64)]
pub struct Repay<'info> {
    #[account(mut, close = admin, seeds = [PREFIX_RECEIPT.as_bytes(), receipt_sn.to_le_bytes().as_ref()], bump)]
    pub receipt: Account<'info, LoanReceipt>,
    #[account(mut)]
    pub lender_balance: Account<'info, UserBalance>,
    #[account(seeds = [PREFIX_CONFIG.as_bytes()], bump)]
    pub config: Account<'info, Config>,
    /// CHECK:
    #[account(mut, constraint = admin.key.to_string() == ADMIN)]
    pub admin: AccountInfo<'info>,
    #[account(seeds = [PREFIX_CCFEE.as_bytes(), receipt.chainid.to_le_bytes().as_ref()], bump)]
    pub cc_fee: Account<'info, CCFee>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
#[instruction(receipt_sn: u64)]
pub struct Liquidate<'info> {
    #[account(mut, close = admin, seeds = [PREFIX_RECEIPT.as_bytes(), receipt_sn.to_le_bytes().as_ref()], bump)]
    pub receipt: Account<'info, LoanReceipt>,
    #[account(seeds = [PREFIX_CONFIG.as_bytes()], bump)]
    pub config: Account<'info, Config>,
    /// CHECK:
    #[account(mut, constraint = admin.key.to_string() == ADMIN)]
    pub admin: AccountInfo<'info>,
    #[account(seeds = [PREFIX_CCFEE.as_bytes(), receipt.chainid.to_le_bytes().as_ref()], bump)]
    pub cc_fee: Account<'info, CCFee>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = payer, space = 8 + 64, seeds = [PREFIX_STATE.as_bytes()], bump)]
    pub global_state: Account<'info, GlobalState>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetConfig<'info> {
    #[account(init_if_needed, payer = payer, space = 8 + 64, seeds = [PREFIX_CONFIG.as_bytes()], bump)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(_chainid: u32)]
pub struct SetCCFee<'info> {
    #[account(init_if_needed, payer = payer, space = 8 + 8, seeds = [PREFIX_CCFEE.as_bytes(), _chainid.to_le_bytes().as_ref()], bump)]
    pub cc_fee: Account<'info, CCFee>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(Default)]
pub struct LoanReceipt {
    pub sn: u64,
    pub borrower: Pubkey,
    pub lender: Pubkey,
    pub source: [u8; 20], 
    pub chainid: u32,
    pub c_sn: u64,
    pub token: [u8; 20],
    pub frozen: u64,
    pub amount: u64,
    pub time: u64,
    pub rate: u64,
}

#[account]
#[derive(Default)]
pub struct Order {
    pub sn: u64,
    pub lender: Pubkey,
    pub balance: u64,
    pub rate: u64,
}

#[account]
#[derive(Default)]
pub struct UserBalance {
    pub amount: u64,
}

#[account]
#[derive(Default)]
pub struct GlobalState {
    pub curr_order_sn: u64,
    pub curr_receipt_sn: u64,
}

#[account]
#[derive(Default)]
pub struct Config {
    pub min_ir: u64,
    pub max_ir: u64,
    pub penalty_ir: u64,
    pub penalty_days: u64,
    pub commission_rate: u64,
    pub cycle: u64,
    pub deadline: u64,
}

#[account]
#[derive(Default)]
pub struct CCFee {
    pub fee: u64,
}

#[event]
pub struct EventPlaceOrder {
    pub order_sn: u64,
    pub lender: Pubkey,
    pub balance: u64,
    pub rate: u64,
}

#[event]
pub struct EventCancelOrder {
    pub order_sn: u64,
    pub lender: Pubkey,
    pub balance: u64,
}

#[event]
pub struct EventCloseOrder {
    pub order_sn: u64,
    pub lender: Pubkey,
}

#[event]
pub struct EventBorrowSuccess {
    pub receipt_sn: u64,
    pub borrower: Pubkey,
    pub lender: Pubkey,
    pub source: [u8; 20],
    pub chainid: u32,
    pub c_sn: u64,
    pub token: [u8; 20],
    pub frozen: u64,
    pub amount: u64,
    pub time: u64,
    pub rate: u64,
    pub order_sn: u64,
    pub order_balance: u64,
}

#[event]
pub struct EventRepaySuccess {
    pub receipt_sn: u64,
    pub borrower: Pubkey,
    pub lender: Pubkey,
    pub amount: u64,
    pub income: u64,
    pub chainid: u32,
    pub c_sn: u64,
    pub source: [u8; 20],
    pub token: [u8; 20],
    pub frozen: u64,
}

#[event]
pub struct EventLiquidate {
    pub receipt_sn: u64,
    pub borrower: Pubkey,
    pub lender: Pubkey,
    pub chainid: u32,
    pub c_sn: u64,
    pub source: [u8; 20],
    pub frozen: u64,
    pub receiver: [u8; 20],
}

pub fn only_admin<'info>(payer: &Signer<'info>) -> Result<()> {
    if payer.key.to_string() != ADMIN {
        return Err(MyError::NoOperationPermission.into());
    }
    Ok(())
}

#[error_code]
pub enum MyError {
    #[msg("insufficient user balance")]
    InsufficientUserBalance,
    #[msg("illegal interest rate")]
    IllegalInterestRate,
    #[msg("no order found")]
    NoOrderFound,
    #[msg("non-zero order balance")]
    NonZeroOrderBalance,
    #[msg("insufficient order balance")]
    InsufficientOrderBalance,
    #[msg("no operation permission")]
    NoOperationPermission,
    #[msg("no receipt found")]
    NoReceiptFound,
    #[msg("deadline not meet")]
    DeadlineNotMeet,
}