-- Change the type back to SERIAL
ALTER TABLE block
    ALTER COLUMN id SET DATA TYPE BIGINT;

-- Reassign ownership of the sequence to the `id` column
ALTER SEQUENCE block_id_seq OWNED BY block.id;

