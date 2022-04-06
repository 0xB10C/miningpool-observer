-- adds information about the entity that sanctioned the addresses

ALTER TABLE sanctioned_utxo
ADD sanctioned_by INT;

ALTER TABLE sanctioned_utxo_scan_info
ADD sanctioned_by INT;

