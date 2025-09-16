mod constants;
mod pb;

use crate::pb::sf::solana::spl::v1::r#type::instruction::Item;
use crate::pb::sf::solana::spl::v1::r#type::{Burn, InitializeMint, InitializedAccount, Instruction, Mint, SplInstructions, Transfer};
use pb::sol::transactions::v1::Transactions as solTransactions;
use std::collections::{HashMap, HashSet};
use substreams::errors::Error;
use prost::Message;

use pb::sf::substreams::foundational_store::v1::ResponseCode;
use crate::pb::sf::substreams::solana::spl::v1::AccountOwner;
use substreams::store::FoundationalStore;

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
fn map_spl_instructions(transactions: solTransactions, foundational_store: FoundationalStore) -> Result<SplInstructions, Error> {
    let mut instructions: Vec<Instruction> = vec![];

    for confirmed_trx in transactions_owned(transactions) {
        let hash = bs58::encode(confirmed_trx.hash()).into_string();

        let mut output_instructions = OutputInstructions::new(hash.clone());

        for instruction in confirmed_trx.compiled_instructions() {
            process_instruction(&mut output_instructions, &instruction);
        }

        instructions.extend(output_instructions.instructions);
    }

    let mut accounts_to_lookup = HashSet::<String>::new();

    for instruction in &instructions {
        if let Some(ref item) = instruction.item {
            match item {
                Item::Transfer(transfer) => {
                    accounts_to_lookup.insert(transfer.from.clone());
                    accounts_to_lookup.insert(transfer.to.clone());
                }
                Item::Mint(mint) => {
                    accounts_to_lookup.insert(mint.to.clone());
                }
                Item::Burn(burn) => {
                    accounts_to_lookup.insert(burn.from.clone());
                }
                _ => {}
            }
        }
    }

    let owners = resolve_account_owners(&foundational_store, &accounts_to_lookup);

    for instruction in &mut instructions {
        if let Some(ref mut item) = instruction.item {
            match item {
                Item::Transfer(ref mut transfer) => {
                    if let Some(from_owner) = owners.get(&transfer.from) {
                        transfer.from_owner = from_owner.clone();
                    }
                    if let Some(to_owner) = owners.get(&transfer.to) {
                        transfer.to_owner = to_owner.clone();
                    }
                }
                Item::Mint(ref mut mint) => {
                    if let Some(to_owner) = owners.get(&mint.to) {
                        mint.to_owner = to_owner.clone();
                    }
                }
                Item::Burn(ref mut burn) => {
                    if let Some(from_owner) = owners.get(&burn.from) {
                        burn.from_owner = from_owner.clone();
                    }
                }
                _ => {}
            }
        }
    }

    Ok(SplInstructions { instructions })
}

fn resolve_account_owners(
    foundational_store: &FoundationalStore,
    accounts: &HashSet<String>,
) -> HashMap<String, String> {
    let mut results = HashMap::with_capacity(accounts.len());
    if accounts.is_empty() {
        return results;
    }

    let account_bytes: Vec<Vec<u8>> = accounts
        .iter()
        .filter_map(|account| bs58::decode(account).into_vec().ok())
        .collect();

    let resp = foundational_store.get_all(&account_bytes);

    for entry in resp.entries {
        let Some(get_response) = entry.response else { continue; };
        if get_response.response != ResponseCode::Found as i32 { continue; }
        let Some(value) = get_response.value else { continue; };
        let Ok(account_owner) = AccountOwner::decode(value.value.as_slice()) else { continue; };

        let account_b58 = bs58::encode(&entry.key).into_string();
        let owner_b58 = bs58::encode(&account_owner.owner).into_string();
        results.insert(account_b58, owner_b58);
    }

    results
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
        SOLANA_TOKEN_PROGRAM_KEG | SOLANA_TOKEN_PROGRAM_ZQB => {
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
    _meta: &TransactionStatusMeta,
    output: &mut OutputInstructions,
) {
    for inner in compile_instruction.inner_instructions() {
        match inner.program_id().to_string().as_ref() {
            SOLANA_TOKEN_PROGRAM_KEG | SOLANA_TOKEN_PROGRAM_ZQB => {
                match process_token_instruction(output, &inner, _meta) {
                    Err(err) => {
                        panic!("trx_hash {} process token instructions!
                         {}", trx_hash, err);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn process_token_instruction(
    output: &mut OutputInstructions,
    instruction: &InstructionView,
    _meta: &TransactionStatusMeta,
) -> Result<(), Error> {
    match TokenInstruction::unpack(&instruction.data()) {
        Err(err) => {
            if let Some(first_byte) = instruction.data().first() {
                if *first_byte > 39 {
                    return Ok(());
                }
            }
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
                if amt > 0 {
                    let source = &instruction.accounts()[0];
                    let destination = &instruction.accounts()[1];

                    output.add(Item::Transfer(Transfer {
                        from: source.to_string(),
                        to: destination.to_string(),
                        amount: amt,
                        from_owner: String::new(),
                        to_owner: String::new(),
                    }));
                }
            }

            TokenInstruction::TransferChecked { amount: amt, .. } => {
                if amt > 0 {
                    let source = &instruction.accounts()[0];
                    let destination = &instruction.accounts()[2];

                    output.add(Item::Transfer(Transfer {
                        from: source.to_string(),
                        to: destination.to_string(),
                        amount: amt,
                        from_owner: String::new(),
                        to_owner: String::new(),
                    }));
                }
            }

            TokenInstruction::MintTo { amount: amt } | TokenInstruction::MintToChecked { amount: amt, .. } => {
                let mint = &instruction.accounts()[0];

                let account_to = &instruction.accounts()[1];
                output.add(Item::Mint(Mint {
                    mint_address: mint.to_string(),
                    to: account_to.to_string(),
                    amount: amt,
                    to_owner: String::new(),
                }));
            }

            TokenInstruction::Burn { amount: amt } | TokenInstruction::BurnChecked { amount: amt, .. } => {
                let mint = &instruction.accounts()[1];
                let account_from = &instruction.accounts()[0];
                output.add(Item::Burn(Burn {
                    mint_address: mint.to_string(),
                    from: account_from.to_string(),
                    amount: amt,
                    from_owner: String::new(),
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
