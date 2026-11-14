use strum::EnumMessage;

/// A response status the view answers with; the discriminant is its code
/// and the message its reason phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumMessage)]
#[repr(u16)]
pub enum Status {
    #[strum(message = "OK")]
    Ok = 200,
    #[strum(message = "Bad Request")]
    BadRequest = 400,
    #[strum(message = "Forbidden")]
    Forbidden = 403,
    #[strum(message = "Not Found")]
    NotFound = 404,
    #[strum(message = "Method Not Allowed")]
    MethodNotAllowed = 405,
    #[strum(message = "Request Header Fields Too Large")]
    HeaderFieldsTooLarge = 431,
}

impl Status {
    /// The numeric status code.
    pub fn code(self) -> u16 {
        self as u16
    }

    /// The status line's reason phrase.
    pub fn reason(self) -> &'static str {
        self.get_message().unwrap_or_default()
    }
}
