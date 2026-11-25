-- Runs once, when the database cluster is first created.
-- pgvector is installed now so that any incompatibility with this PostgreSQL
-- version shows up immediately, long before the agent needs it.
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pgxila;
