CREATE TABLE authorized_users (
  email VARCHAR(100) NOT NULL UNIQUE PRIMARY KEY,
  telegram_userid bigint,
  created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);
