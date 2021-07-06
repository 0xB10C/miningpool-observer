-- stores information about the node used to generate the blocktemplate

CREATE TABLE IF NOT EXISTS node_info (
    id             INT,
    version        TEXT NOT NULL,
    PRIMARY KEY (id)
);

INSERT INTO node_info VALUES (0, '-') ON CONFLICT DO NOTHING;