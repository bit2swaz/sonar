use std::error::Error;

use anchor_lang::solana_program::{account_info::AccountInfo, entrypoint::ProgramResult};
use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas};
use solana_program_test::{processor, ProgramTest};
use solana_sdk::{
    account::Account,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use sonar_program::{
    accounts as sonar_accounts, instruction as sonar_instruction, CallbackParams, RequestMetadata,
    RequestStatus, ResultAccount,
};

fn process_sonar<'a, 'b, 'c, 'd>(
    program_id: &'a Pubkey,
    accounts: &'b [AccountInfo<'c>],
    instruction_data: &'d [u8],
) -> ProgramResult {
    let accounts: &'c [AccountInfo<'c>] = unsafe { std::mem::transmute(accounts) };
    sonar_program::entry(program_id, accounts, instruction_data)
}

#[tokio::test]
async fn end_to_end_historical_average_flow_works() -> Result<(), Box<dyn Error>> {
    std::env::set_var("SP1_PROVER", "mock");

    let prover = Keypair::new();
    let request_id = [42u8; 32];
    let balances = vec![120u64, 180u64, 360u64, 540u64];
    let expected_avg = 300u64;
    let callback_program = anchor_lang::solana_program::system_program::ID;

    let (request_metadata, request_bump) =
        Pubkey::find_program_address(&[b"request", request_id.as_ref()], &sonar_program::id());
    let (result_account, result_bump) =
        Pubkey::find_program_address(&[b"result", request_id.as_ref()], &sonar_program::id());

    let mut request_metadata_data = Vec::new();
    RequestMetadata {
        request_id,
        payer: Pubkey::new_unique(),
        callback_program,
        result_account,
        computation_id: sonar_program::HISTORICAL_AVG_COMPUTATION_ID,
        deadline: 1_000_000,
        fee: 1_000_000,
        status: RequestStatus::Pending,
        completed_at: None,
        bump: request_bump,
    }
    .try_serialize(&mut request_metadata_data)?;
    request_metadata_data.resize(RequestMetadata::LEN, 0);

    let mut result_account_data = Vec::new();
    ResultAccount {
        request_id,
        result: Vec::new(),
        is_set: false,
        written_at: None,
        bump: result_bump,
    }
    .try_serialize(&mut result_account_data)?;
    result_account_data.resize(ResultAccount::LEN, 0);

    let mut program_test = ProgramTest::new(
        "sonar_program",
        sonar_program::id(),
        processor!(process_sonar),
    );
    program_test.add_account(
        prover.pubkey(),
        Account {
            lamports: 1_000_000_000,
            data: Vec::new(),
            owner: anchor_lang::solana_program::system_program::ID,
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        request_metadata,
        Account {
            lamports: 2_000_000,
            data: request_metadata_data,
            owner: sonar_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );
    program_test.add_account(
        result_account,
        Account {
            lamports: 1_000_000,
            data: result_account_data,
            owner: sonar_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let context = program_test.start_with_context().await;

    let historical_avg_id = sonar_prover::historical_avg_computation_id()?;
    assert_eq!(
        historical_avg_id,
        sonar_program::HISTORICAL_AVG_COMPUTATION_ID
    );

    let encoded_balances = bincode::serialize(&balances)?;
    let proof_input = encoded_balances.clone();
    let proof_computation_id = historical_avg_id;
    let (proof, result, public_inputs) = tokio::task::spawn_blocking(move || {
        sonar_prover::prove(&proof_computation_id, &proof_input)
    })
    .await??;
    assert_eq!(
        u64::from_le_bytes(result.as_slice().try_into()?),
        expected_avg
    );
    assert_eq!(public_inputs, expected_avg.to_le_bytes().to_vec());

    let callback_ix = Instruction {
        program_id: sonar_program::id(),
        accounts: sonar_accounts::Callback {
            request_metadata,
            result_account,
            prover: prover.pubkey(),
            callback_program,
        }
        .to_account_metas(None),
        data: sonar_instruction::Callback {
            params: CallbackParams {
                proof,
                public_inputs: vec![public_inputs],
                result,
            },
        }
        .data(),
    };

    let callback_tx = Transaction::new_signed_with_payer(
        &[callback_ix],
        Some(&context.payer.pubkey()),
        &[&context.payer, &prover],
        context.banks_client.get_latest_blockhash().await?,
    );
    context
        .banks_client
        .process_transaction(callback_tx)
        .await?;

    let request_metadata_account = context
        .banks_client
        .get_account(request_metadata)
        .await?
        .expect("request metadata should exist");
    let mut request_metadata_bytes = request_metadata_account.data.as_slice();
    let request_metadata_value = RequestMetadata::try_deserialize(&mut request_metadata_bytes)?;
    assert!(matches!(
        request_metadata_value.status,
        RequestStatus::Completed
    ));

    let result_account_data = context
        .banks_client
        .get_account(result_account)
        .await?
        .expect("result account should exist");
    let mut result_account_bytes = result_account_data.data.as_slice();
    let result_account_value = ResultAccount::try_deserialize(&mut result_account_bytes)?;
    assert!(result_account_value.is_set);
    assert_eq!(
        u64::from_le_bytes(result_account_value.result.as_slice().try_into()?),
        expected_avg
    );

    Ok(())
}
