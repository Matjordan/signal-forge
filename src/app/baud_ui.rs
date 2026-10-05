use super::*;

// Add standard rates here to expose them in both serial configuration controls.
const COMMON_BAUD_RATES: &[u32] = &[
    300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
];
const INVALID_BAUD: &str = "Baud rate must be a whole number from 1 to 4294967295.";

pub(super) struct BaudControl {
    custom: bool,
    text: String,
    observed: u32,
}
impl BaudControl {
    pub(super) fn new(baud: u32) -> Self {
        Self {
            custom: !COMMON_BAUD_RATES.contains(&baud),
            text: baud.to_string(),
            observed: baud,
        }
    }
    fn parse(text: &str) -> Result<u32, &'static str> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(INVALID_BAUD);
        }
        text.parse::<u32>()
            .ok()
            .filter(|baud| *baud > 0)
            .ok_or(INVALID_BAUD)
    }
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        if self.custom {
            Self::parse(&self.text).map(|_| ())
        } else {
            Ok(())
        }
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui, baud: &mut u32) {
        if self.observed != *baud {
            *self = Self::new(*baud);
        }
        ui.label("Baud");
        egui::ComboBox::from_id_salt("baud")
            .width(75.0)
            .selected_text(if self.custom {
                match Self::parse(&self.text) {
                    Ok(value) => format!("{value} (custom)"),
                    Err(_) => "Custom (invalid)".into(),
                }
            } else {
                baud.to_string()
            })
            .show_ui(ui, |ui| {
                for &rate in COMMON_BAUD_RATES {
                    if ui
                        .selectable_label(!self.custom && *baud == rate, rate.to_string())
                        .clicked()
                    {
                        *baud = rate;
                        *self = Self::new(rate);
                    }
                }
                if ui.selectable_label(self.custom, "Custom…").clicked() {
                    self.custom = true;
                }
            });
        if self.custom {
            ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .desired_width(95.0)
                    .hint_text("Baud rate"),
            );
            match Self::parse(&self.text) {
                Ok(value) => *baud = value,
                Err(error) => {
                    ui.colored_label(theme::ERROR, error);
                }
            }
        }
        self.observed = *baud;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_baud_validation() {
        for text in ["", "0", "-1", "1.5", "abc", "4294967296", "+9600"] {
            assert!(BaudControl::parse(text).is_err(), "{text}");
        }
        for value in [1, 14400, 19200, u32::MAX] {
            assert_eq!(BaudControl::parse(&value.to_string()), Ok(value));
        }
    }
    #[test]
    fn saved_rates_initialize_without_substitution() {
        for baud in [9600, 19200, 14400, 5_000_000] {
            let control = BaudControl::new(baud);
            assert_eq!(control.custom, !COMMON_BAUD_RATES.contains(&baud));
            assert_eq!(BaudControl::parse(&control.text), Ok(baud));
            assert!(control.validate().is_ok());
        }
    }
}
