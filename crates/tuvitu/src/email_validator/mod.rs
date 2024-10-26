use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

static EMAIL_REGEX: Lazy<Regex> =
	Lazy::new(|| Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$").unwrap());

#[derive(Error, Debug, PartialEq)]
pub enum EmailError {
	#[error("Invalid email format")]
	InvalidFormat,
	#[error("Domain not permitted")]
	DomainNotPermitted,
	#[error("Local part exceeds maximum length of 64 characters")]
	LocalPartTooLong,
	#[error("Domain exceeds maximum length of 255 characters")]
	DomainTooLong,
	#[error("Email contains invalid characters")]
	InvalidCharacters,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KnownDomain {
	Gmail,
	Yahoo,
	Outlook,
	Hotmail,
}

impl KnownDomain {
	fn domain_str(&self) -> &'static str {
		match self {
			KnownDomain::Gmail => "gmail.com",
			KnownDomain::Yahoo => "yahoo.com",
			KnownDomain::Outlook => "outlook.com",
			KnownDomain::Hotmail => "hotmail.com",
		}
	}
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmailDomain {
	Known(KnownDomain),
	Custom(String),
}

impl fmt::Display for EmailDomain {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			EmailDomain::Known(known) => write!(f, "{}", known.domain_str()),
			EmailDomain::Custom(domain) => write!(f, "{}", domain),
		}
	}
}

#[derive(Debug, Clone)]
pub struct DomainRegistry {
	permitted_known_domains: HashSet<KnownDomain>,
	permitted_custom_domains: HashSet<String>,
	allow_any_custom: bool,
}

impl Default for DomainRegistry {
	fn default() -> Self {
		Self::new()
	}
}

impl DomainRegistry {
	pub fn new() -> Self {
		Self {
			permitted_known_domains: HashSet::new(),
			permitted_custom_domains: HashSet::new(),
			allow_any_custom: false,
		}
	}

	pub fn permit_known_domain(mut self, domain: KnownDomain) -> Self {
		self.permitted_known_domains.insert(domain);
		self
	}

	pub fn permit_custom_domain(mut self, domain: String) -> Self {
		self.permitted_custom_domains.insert(domain.to_lowercase());
		self
	}

	pub fn allow_any_custom_domain(mut self, allow: bool) -> Self {
		self.allow_any_custom = allow;
		self
	}

	fn is_domain_permitted(&self, domain_str: &str) -> bool {
		let domain_str = domain_str.to_lowercase();

		// Check known domains
		for known in &self.permitted_known_domains {
			if known.domain_str() == domain_str {
				return true;
			}
		}

		// Check custom domains
		if self.permitted_custom_domains.contains(&domain_str) {
			return true;
		}

		// Check if any custom domain is allowed
		self.allow_any_custom && Self::is_valid_custom_domain(&domain_str)
	}

	fn is_valid_custom_domain(domain: &str) -> bool {
		!domain.is_empty() && domain.contains('.') && domain.len() <= 255 && !domain.starts_with('.') && !domain.ends_with('.') && !domain.contains("..")
	}
}

#[derive(Debug, Clone)]
pub struct EmailValidator {
	domain_registry: DomainRegistry,
}

impl Default for EmailValidator {
	fn default() -> Self {
		Self::new()
	}
}

impl EmailValidator {
	pub fn new() -> Self {
		Self {
			domain_registry: DomainRegistry::new(),
		}
	}

	pub fn with_domain_registry(domain_registry: DomainRegistry) -> Self {
		Self { domain_registry }
	}

	pub fn parse(&self, s: &str) -> Result<Email, EmailError> {
		// Basic format validation
		if !EMAIL_REGEX.is_match(s) {
			return Err(EmailError::InvalidFormat);
		}

		let parts: Vec<&str> = s.split('@').collect();
		if parts.len() != 2 {
			return Err(EmailError::InvalidFormat);
		}

		let local_part = parts[0].to_string();
		let domain_str = parts[1].to_lowercase();

		// Validate lengths
		if local_part.len() > 64 {
			return Err(EmailError::LocalPartTooLong);
		}
		if domain_str.len() > 255 {
			return Err(EmailError::DomainTooLong);
		}

		// Check if domain is permitted
		if !self.domain_registry.is_domain_permitted(&domain_str) {
			return Err(EmailError::DomainNotPermitted);
		}

		// Determine domain type
		let domain = match domain_str.as_str() {
			"gmail.com" => EmailDomain::Known(KnownDomain::Gmail),
			"yahoo.com" => EmailDomain::Known(KnownDomain::Yahoo),
			"outlook.com" => EmailDomain::Known(KnownDomain::Outlook),
			"hotmail.com" => EmailDomain::Known(KnownDomain::Hotmail),
			custom => EmailDomain::Custom(custom.to_string()),
		};

		Ok(Email { local_part, domain })
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email {
	local_part: String,
	domain: EmailDomain,
}

impl Email {
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

// ... [Previous code remains the same until the tests module] ...

#[cfg(test)]
mod tests {
	use super::*;

	// Helper functions for creating validators
	fn create_gmail_only_validator() -> EmailValidator {
		let registry = DomainRegistry::new().permit_known_domain(KnownDomain::Gmail);
		EmailValidator::with_domain_registry(registry)
	}

	fn create_custom_only_validator() -> EmailValidator {
		let registry = DomainRegistry::new().permit_custom_domain("example.com".to_string());
		EmailValidator::with_domain_registry(registry)
	}

	fn create_permissive_validator() -> EmailValidator {
		let registry = DomainRegistry::new()
			.permit_known_domain(KnownDomain::Gmail)
			.permit_known_domain(KnownDomain::Yahoo)
			.allow_any_custom_domain(true);
		EmailValidator::with_domain_registry(registry)
	}

	mod domain_permission_tests {
		use super::*;

		#[test]
		fn test_gmail_only_validator() {
			let validator = create_gmail_only_validator();

			assert!(validator.parse("test@gmail.com").is_ok());
			assert_eq!(validator.parse("test@yahoo.com"), Err(EmailError::DomainNotPermitted));
			assert_eq!(validator.parse("test@example.com"), Err(EmailError::DomainNotPermitted));
		}

		#[test]
		fn test_custom_only_validator() {
			let validator = create_custom_only_validator();

			assert!(validator.parse("test@example.com").is_ok());
			assert_eq!(validator.parse("test@gmail.com"), Err(EmailError::DomainNotPermitted));
			assert_eq!(validator.parse("test@other.com"), Err(EmailError::DomainNotPermitted));
		}

		#[test]
		fn test_permissive_validator() {
			let validator = create_permissive_validator();

			assert!(validator.parse("test@gmail.com").is_ok());
			assert!(validator.parse("test@yahoo.com").is_ok());
			assert!(validator.parse("test@custom-domain.com").is_ok());
		}
	}

	mod format_validation_tests {
		use super::*;

		#[test]
		fn test_valid_local_parts() {
			let validator = create_permissive_validator();
			let valid_locals = vec![
				"simple",
				"very.common",
				"disposable.style.email.with+symbol",
				"other.email-with-hyphen",
				"fully-qualified-domain",
				"user.name+tag",
				"x@example.com",
				"_______",
				"email",
				"firstname.lastname",
				"email",
				"1234567890",
				"email-with-dash",
			];

			for local in valid_locals {
				let email = format!("{}@gmail.com", local);
				assert!(validator.parse(&email).is_ok(), "Failed for local part: {}", local);
			}
		}

		#[test]
		fn test_invalid_local_parts() {
			let validator = create_permissive_validator();
			let invalid_locals = vec![
				"",
				"Abc.example.com",
				"just\"not\"right@example.com",
				"this is\"not\\allowed@example.com",
				"this\\ still\\\"not\\allowed@example.com",
				".leading-dot",
				"trailing-dot.",
				"double..dot",
				" space-in-local",
				"trailing-space ",
			];

			for local in invalid_locals {
				let email = format!("{}@gmail.com", local);
				assert!(validator.parse(&email).is_err(), "Should fail for local part: {}", local);
			}
		}

		#[test]
		fn test_valid_domains() {
			let validator = create_permissive_validator();
			let valid_domains = vec!["example.com", "example-domain.com", "dept.example.org", "long.domain.with.many.parts.com"];

			for domain in valid_domains {
				let email = format!("test@{}", domain);
				assert!(validator.parse(&email).is_ok(), "Failed for domain: {}", domain);
			}
		}

		#[test]
		fn test_invalid_domains() {
			let validator = create_permissive_validator();
			let invalid_domains = vec![
				"",
				".com",
				"example.",
				".example.com",
				"example..com",
				"-example.com",
				"example-.com",
				"example.com-",
				"example.-com",
				"@example.com",
			];

			for domain in invalid_domains {
				let email = format!("test@{}", domain);
				assert!(validator.parse(&email).is_err(), "Should fail for domain: {}", domain);
			}
		}
	}

	mod length_validation_tests {
		use super::*;

		#[test]
		fn test_local_part_length_limits() {
			let validator = create_permissive_validator();

			// Test maximum valid length (64 characters)
			let max_local = "a".repeat(64);
			let valid_email = format!("{}@gmail.com", max_local);
			assert!(validator.parse(&valid_email).is_ok());

			// Test exceeding maximum length
			let too_long_local = "a".repeat(65);
			let invalid_email = format!("{}@gmail.com", too_long_local);
			assert_eq!(validator.parse(&invalid_email), Err(EmailError::LocalPartTooLong));
		}

		#[test]
		fn test_domain_length_limits() {
			let validator = create_permissive_validator();

			// Test maximum valid length (255 characters)
			let max_domain = format!("{}.com", "a".repeat(251));
			let valid_email = format!("test@{}", max_domain);
			assert!(validator.parse(&valid_email).is_ok());

			// Test exceeding maximum length
			let too_long_domain = format!("{}.com", "a".repeat(252));
			let invalid_email = format!("test@{}", too_long_domain);
			assert_eq!(validator.parse(&invalid_email), Err(EmailError::DomainTooLong));
		}
	}

	mod email_display_tests {
		use super::*;

		#[test]
		fn test_email_display() {
			let validator = create_gmail_only_validator();
			let email = validator.parse("test@gmail.com").unwrap();
			assert_eq!(email.to_string(), "test@gmail.com");
			assert_eq!(email.as_str(), "test@gmail.com");
		}

		#[test]
		fn test_email_parts_access() {
			let validator = create_gmail_only_validator();
			let email = validator.parse("local-part@gmail.com").unwrap();

			assert_eq!(email.local_part(), "local-part");
			assert_eq!(email.domain(), &EmailDomain::Known(KnownDomain::Gmail));
		}
	}

	mod domain_registry_tests {
		use super::*;

		#[test]
		fn test_domain_registry_builder() {
			let registry = DomainRegistry::new()
				.permit_known_domain(KnownDomain::Gmail)
				.permit_known_domain(KnownDomain::Yahoo)
				.permit_custom_domain("example.com".to_string())
				.allow_any_custom_domain(true);

			assert!(registry.is_domain_permitted("gmail.com"));
			assert!(registry.is_domain_permitted("yahoo.com"));
			assert!(registry.is_domain_permitted("example.com"));
			assert!(registry.is_domain_permitted("any-custom-domain.com"));
		}

		#[test]
		fn test_case_insensitive_domains() {
			let registry = DomainRegistry::new()
				.permit_known_domain(KnownDomain::Gmail)
				.permit_custom_domain("Example.Com".to_string());

			assert!(registry.is_domain_permitted("gmail.com"));
			assert!(registry.is_domain_permitted("Gmail.Com"));
			assert!(registry.is_domain_permitted("GMAIL.COM"));
			assert!(registry.is_domain_permitted("example.com"));
			assert!(registry.is_domain_permitted("EXAMPLE.COM"));
		}
	}

	mod edge_cases {
		use super::*;

		#[test]
		fn test_edge_case_emails() {
			let validator = create_permissive_validator();

			// Test empty string
			assert!(validator.parse("").is_err());

			// Test single character local part
			assert!(validator.parse("a@gmail.com").is_ok());

			// Test multiple @ symbols
			assert!(validator.parse("test@foo@gmail.com").is_err());

			// Test special characters
			assert!(validator.parse("test!#$%&'*+-/=?^_`{|}~@gmail.com").is_ok());

			// Test quoted strings (should fail as we don't support them)
			assert!(validator.parse("\"test\"@gmail.com").is_err());
		}
	}
}
