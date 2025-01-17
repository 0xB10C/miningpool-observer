-- Create a block.id sequence with a random start position.
-- This allows importing old backups.
DO $$
DECLARE
    random_start BIGINT;
BEGIN
    -- Generate a random start value for the sequence
    random_start := (floor(random() * 9223372036854775807)::BIGINT);

    -- Create the sequence with the random start value
    EXECUTE format('CREATE SEQUENCE IF NOT EXISTS block_id_seq_random START %s CYCLE', random_start);
END $$;

ALTER TABLE block
    ALTER COLUMN id SET DATA TYPE BIGINT, -- Change the column to BIGINT
    ALTER COLUMN id SET DEFAULT nextval('block_id_seq_random');  -- Use the custom sequence

-- Reassign ownership of the sequence to the `id` column
ALTER SEQUENCE block_id_seq_random OWNED BY block.id;

ALTER TABLE conflicting_transactions
    ALTER COLUMN block_id SET DATA TYPE BIGINT;
ALTER TABLE debug_template_selection
    ALTER COLUMN block_id SET DATA TYPE BIGINT;
ALTER TABLE transaction_only_in_block
    ALTER COLUMN block_id SET DATA TYPE BIGINT;
ALTER TABLE transaction_only_in_template
    ALTER COLUMN block_id SET DATA TYPE BIGINT;
ALTER TABLE sanctioned_transaction_info
    ALTER COLUMN block_id SET DATA TYPE BIGINT;
