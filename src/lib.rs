mod constants;
mod pb;

use crate::pb::sf::solana::spl::v1::r#type::instruction::Item;
use crate::pb::sf::solana::spl::v1::r#type::{
    Burn, InitializeMint, InitializedAccount, Instruction, Mint, SplInstructions, Transfer,
};

use pb::sol::transactions::v1::Transactions as solTransactions;
use std::ops::Div;
use substreams::errors::Error;
use substreams::log;
use substreams_solana::block_view::InstructionView;
use substreams_solana::pb::sf::solana::r#type::v1::{ConfirmedTransaction, TransactionStatusMeta};
use substreams_solana_program_instructions::token_instruction_2022::TokenInstruction;

pub const SOLANA_TOKEN_PROGRAM_KEG: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const SOLANA_TOKEN_PROGRAM_ZQB: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

struct OutputInstructions {
    transaction_hash: String,
    ordinal: i64,
    instructions: Vec<Instruction>,
}

impl OutputInstructions {
    pub fn new(transaction_hash: String) -> Self {
        Self {
            transaction_hash,
            ordinal: 0,
            instructions: vec![],
        }
    }

    pub fn add(&mut self, item: Item) {
        self.instructions.push(Instruction {
            transaction_hash: self.transaction_hash.to_string(),
            instruction_id: self.transaction_hash.to_string() + "-" + &self.ordinal.to_string(),
            item: Some(item),
        });

        self.ordinal += 1;
    }
}

#[substreams::handlers::map]
fn map_spl_instructions(params: String, transactions: solTransactions) -> Result<SplInstructions, Error> {
    let mut instructions: Vec<Instruction> = vec![];
    log::info!("map_spl_instructions");
    for confirmed_trx in transactions_owned(transactions) {
        let hash = bs58::encode(confirmed_trx.hash()).into_string();

        let mut output_instructions = OutputInstructions::new(hash.clone());

        for instruction in confirmed_trx.compiled_instructions() {
            process_instruction(&mut output_instructions, &instruction);
        }

        instructions.extend(output_instructions.instructions);
    }
    Ok(SplInstructions { instructions })
}

/// Iterates over successful transactions in given block and take ownership.
fn transactions_owned(transactions: solTransactions) -> impl Iterator<Item = ConfirmedTransaction> {
    transactions.transactions.into_iter().filter(|trx| -> bool {
        if let Some(meta) = &trx.meta {
            return meta.err.is_none();
        }
        false
    })
}

fn process_instruction(output: &mut OutputInstructions, compile_instruction: &InstructionView) {
    let trx_hash = &bs58::encode(compile_instruction.transaction().hash()).into_string();
    match compile_instruction.program_id().to_string().as_ref() {
        SOLANA_TOKEN_PROGRAM_KEG => {
            match process_token_instruction(output, compile_instruction, compile_instruction.meta()) {
                Err(err) => {
                    panic!("trx_hash {} process token instructions: {}", trx_hash, err);
                }
                _ => {}
            }
        }
        SOLANA_TOKEN_PROGRAM_ZQB => {
            match process_token_instruction(output, compile_instruction, compile_instruction.meta()) {
                Err(err) => {
                    panic!("trx_hash {} process token instructions: {}", trx_hash, err);
                }
                _ => {}
            }
        }
        _ => {
            process_inner_instruction(compile_instruction, trx_hash, compile_instruction.meta(), output);
        }
    }
}

fn process_inner_instruction(
    compile_instruction: &InstructionView,
    trx_hash: &String,
    meta: &TransactionStatusMeta,
    output: &mut OutputInstructions,
) {
    for inner in compile_instruction.inner_instructions() {
        match inner.program_id().to_string().as_ref() {
            SOLANA_TOKEN_PROGRAM_KEG => match process_token_instruction(output, &inner, meta) {
                Err(err) => {
                    panic!("trx_hash {} process token instructions {}", trx_hash, err);
                }
                _ => {}
            },
            SOLANA_TOKEN_PROGRAM_ZQB => match process_token_instruction(output, &inner, meta) {
                Err(err) => {
                    panic!("trx_hash {} process token instructions {}", trx_hash, err);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn process_token_instruction(
    output: &mut OutputInstructions,
    instruction: &InstructionView,
    meta: &TransactionStatusMeta,
) -> Result<(), Error> {
    match TokenInstruction::unpack(&instruction.data()) {
        Err(err) => {
            return Err(anyhow::anyhow!("unpacking token instruction: {}", err));
        }
        Ok(token_instruction) => match token_instruction {
            TokenInstruction::InitializeMint {
                decimals,
                mint_authority,
                freeze_authority,
            } => {
                let mint = &instruction.accounts()[0];
                let freeze_authority = freeze_authority.map_or("".to_string(), |a| bs58::encode(a).into_string());
                output.add(Item::InitializeMint(InitializeMint {
                    mint_address: mint.to_string(),
                    decimals: decimals as u64,
                    mint_authority: bs58::encode(mint_authority).into_string(),
                    freeze_authority,
                }));
            }
            TokenInstruction::InitializeMint2 {
                decimals,
                mint_authority,
                freeze_authority,
            } => {
                let mint = &instruction.accounts()[0];
                let freeze_authority = freeze_authority.map_or("".to_string(), |a| bs58::encode(a).into_string());
                output.add(Item::InitializeMint(InitializeMint {
                    mint_address: mint.to_string(),
                    decimals: decimals as u64,
                    mint_authority: bs58::encode(mint_authority).into_string(),
                    freeze_authority,
                }));
            }
            TokenInstruction::Transfer { amount: amt } => {
                let source = &instruction.accounts()[0];
                // let source = &accounts[inst_accounts[0] as usize];
                let destination = &instruction.accounts()[1];
                // let destination = &accounts[inst_accounts[1] as usize];

                output.add(Item::Transfer(Transfer {
                    from: source.to_string(),
                    to: destination.to_string(),
                    amount: amt,
                }));
            }

            TokenInstruction::TransferChecked { amount: amt, .. } => {
                let mint = &instruction.accounts()[1];
                let source = &instruction.accounts()[0];
                let destination = &instruction.accounts()[2];

                output.add(Item::Transfer(Transfer {
                    from: source.to_string(),
                    to: destination.to_string(),
                    amount: amt,
                }));
            }

            TokenInstruction::MintTo { amount: amt } | TokenInstruction::MintToChecked { amount: amt, .. } => {
                let mint = &instruction.accounts()[0];

                let account_to = &instruction.accounts()[1];
                output.add(Item::Mint(Mint {
                    mint_address: mint.to_string(),
                    to: account_to.to_string(),
                    amount: amt,
                }));
            }

            TokenInstruction::Burn { amount: amt } | TokenInstruction::BurnChecked { amount: amt, .. } => {
                let mint = &instruction.accounts()[1];
                let account_from = &instruction.accounts()[0];
                output.add(Item::Burn(Burn {
                    mint_address: mint.to_string(),
                    from: account_from.to_string(),
                    amount: amt,
                }));
            }
            TokenInstruction::InitializeAccount {} => {
                let mint = &instruction.accounts()[1];

                let account = &instruction.accounts()[0];
                let owner = &instruction.accounts()[2];

                output.add(Item::InitializedAccount(InitializedAccount {
                    account: account.to_string(),
                    mint_address: mint.to_string(),
                    owner: owner.to_string(),
                }));
            }
            TokenInstruction::InitializeAccount2 { owner: ow } | TokenInstruction::InitializeAccount3 { owner: ow } => {
                let mint = &instruction.accounts()[1];

                let account = &instruction.accounts()[0];

                output.add(Item::InitializedAccount(InitializedAccount {
                    account: account.to_string(),
                    mint_address: mint.to_string(),
                    owner: bs58::encode(ow).into_string(),
                }));
            }
            _ => {}
        },
    }

    Ok(())
}
