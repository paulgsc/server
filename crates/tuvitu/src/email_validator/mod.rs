use std::fmt;
use regex::Regex;
use thiserror::Error;
use once_cell::sync::Lazy;

static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$").unwrap()
});

#[derive(Error, Debug, PartialEq)]
pub enum EmailError {
    #[error("Invalid email format")]
    InvalidFormat,
    #[error("Invalid or unsupported domain")]
    InvalidDomain,
    #[error("Local part exceeds maximum length of 64 characters")]
    LocalPartTooLong,
    #[error("Domain exceeds maximum length of 255 characters")]
    DomainTooLong,
    #[error("Email contains invalid characters")]
    InvalidCharacters,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmailDomain {
    Gmail,
    Yahoo,
    Outlook,
    Hotmail,
    Custom(String),
}

impl EmailDomain {
    pub fn is_permitted(&self) -> bool {
        match self {
            EmailDomain::Gmail => true,
            EmailDomain::Yahoo => true,
            EmailDomain::Outlook => true,
            EmailDomain::Hotmail => true,
            EmailDomain::Custom(domain) => {
                // Basic validation for custom domains
                !domain.is_empty() && 
                domain.contains('.') && 
                domain.len() <= 255 &&
                !domain.starts_with('.') &&
                !domain.ends_with('.')
            }
        }
    }

    pub fn from_str(domain: &str) -> Result<Self, EmailError> {
        if domain.len() > 255 {
            return Err(EmailError::DomainTooLong);
        }

        match domain.to_lowercase().as_str() {
            "gmail.com" => Ok(EmailDomain::Gmail),
            "yahoo.com" => Ok(EmailDomain::Yahoo),
            "outlook.com" => Ok(EmailDomain::Outlook),
            "hotmail.com" => Ok(EmailDomain::Hotmail),
            custom => {
                if custom.contains("..") {
                    Err(EmailError::InvalidDomain)
                } else {
                    Ok(EmailDomain::Custom(custom.to_string()))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email {
    local_part: String,
    domain: EmailDomain,
}

impl Email {
    pub fn parse(s: String) -> Result<Self, EmailError> {
        // Basic format validation
        if !EMAIL_REGEX.is_match(&s) {
            return Err(EmailError::InvalidFormat);
        }

        // Split the email into local part and domain
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return Err(EmailError::InvalidFormat);
        }

        let local_part = parts[0].to_string();
        let domain_str = parts[1];

        // Validate lengths
        if local_part.len() > 64 {
            return Err(EmailError::LocalPartTooLong);
        }

        // Additional local part validation
        if local_part.starts_with('.') || local_part.ends_with('.') {
            return Err(EmailError::InvalidFormat);
        }

        // Ensure the domain is permitted
        let domain = EmailDomain::from_str(domain_str)?;
        if !domain.is_permitted() {
            return Err(EmailError::InvalidDomain);
        }

        Ok(Self { local_part, domain })
    }

    pub fn local_part(&self) -> &str {
        &self.local_part
    }

    pub fn domain(&self) -> &EmailDomain {
        &self.domain
    }

    pub fn as_str(&self) -> String {
        format!("{}@{}", self.local_part, self.domain)
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local_part, self.domain)
    }
}

impl fmt::Display for EmailDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmailDomain::Gmail => write!(f, "gmail.com"),
            EmailDomain::Yahoo => write!(f, "yahoo.com"),
            EmailDomain::Outlook => write!(f, "outlook.com"),
            EmailDomain::Hotmail => write!(f, "hotmail.com"),
            EmailDomain::Custom(domain) => write!(f, "{}", domain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_emails() {
        let valid_emails = vec![
            "test@gmail.com",
            "user.name@yahoo.com",
            "test.email+suffix@outlook.com",
            "user@hotmail.com",
            "test@custom-domain.com",
        ];

        for email in valid_emails {
            assert!(Email::parse(email.to_string()).is_ok());
        }
    }

    #[test]
    fn test_invalid_emails() {
        let invalid_emails = vec![
            "@gmail.com",
            "test@",
            "test",
            "test@.com",
            "test@domain..com",
            ".test@domain.com",
            "test.@domain.com",
        ];

        for email in invalid_emails {
            assert!(Email::parse(email.to_string()).is_err());
        }
    }

    #[test]
    fn test_email_domain_permitted() {
        let gmail = EmailDomain::Gmail;
        assert!(gmail.is_permitted());

        let custom = EmailDomain::Custom("example.com".to_string());
        assert!(custom.is_permitted());

        let invalid_custom = EmailDomain::Custom("".to_string());
        assert!(!invalid_custom.is_permitted());
    }

    #[test]
    fn test_email_length_limits() {
        let long_local_part = format!("{}@gmail.com", "a".repeat(65));
        assert_eq!(
            Email::parse(long_local_part),
            Err(EmailError::LocalPartTooLong)
        );

        let long_domain = format!("test@{}", "a".repeat(256));
        assert_eq!(
            Email::parse(long_domain),
            Err(EmailError::DomainTooLong)
        );
    }

    #[test]
    fn test_email_display() {
        let email = Email::parse("test@gmail.com".to_string()).unwrap();
        assert_eq!(email.to_string(), "test@gmail.com");
    }
}
