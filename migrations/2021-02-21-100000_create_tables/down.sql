DROP TABLE block CASCADE;
DROP TABLE transaction CASCADE;
DROP TABLE transaction_only_in_block;
DROP TABLE transaction_only_in_template;
DROP TABLE sanctioned_transaction_info;
DROP TABLE conflicting_transactions;

DROP INDEX block_height_desc_index;