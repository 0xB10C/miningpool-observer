-- stores the most recent set of (OFAC) sanctioned addresses
-- downloaded and overwritten on daemon start-up and updated periodically
-- queried by the daemon when processing transactions

CREATE TABLE IF NOT EXISTS sanctioned_addresses (
    address TEXT NOT NULL,
    PRIMARY KEY (address)
);

