-- Add migration script here

CREATE TABLE images (
    id INTEGER PRIMARY KEY NOT NULL,
    author VARCHAR(255) NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    hash BLOB NOT NULL,
    path VARCHAR(255) NOT NULL,
    url VARCHAR(512) NOT NULL,
    mime_type VARCHAR(255) NOT NULL
)
