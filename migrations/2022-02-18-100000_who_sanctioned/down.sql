-- reverts up.sql (adds information about the entity that sanctioned the addresses)

ALTER TABLE sanctioned_utxo
DROP sanctioned_by;

ALTER TABLE sanctioned_utxo_scan_info
DROP sanctioned_by;
