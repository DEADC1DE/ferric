//! Bounded floating-point arithmetic for the supported subset of $calc.

pub(super) fn compute(input: &str) -> Option<f64> {
    let mut p = Parser {
        input: input.as_bytes(),
        pos: 0,
        depth: 0,
        quotient_on_zero: false,
    };
    let result = p.expr()?;
    p.space();
    (p.pos == p.input.len() && result.is_finite()).then_some(result)
}

/// The scalar path of `/set`: a division by zero gives a quotient of 0 instead of no answer.
pub(super) fn compute_scalar(input: &str) -> Option<f64> {
    let mut p = Parser {
        input: input.as_bytes(),
        pos: 0,
        depth: 0,
        quotient_on_zero: true,
    };
    let result = p.expr()?;
    p.space();
    (p.pos == p.input.len() && result.is_finite()).then_some(result)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    depth: usize,
    quotient_on_zero: bool,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self
            .input
            .get(self.pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.pos += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.space();
        if self.input.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.take(b'+') {
                value += self.term()?;
            } else if self.take(b'-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.atom()?;
        loop {
            if self.take(b'*') {
                value *= self.atom()?;
            } else if self.take(b'/') {
                let rhs = self.atom()?;
                if rhs == 0.0 {
                    if !self.quotient_on_zero {
                        return None;
                    }
                    value = 0.0;
                } else {
                    value /= rhs;
                }
            } else {
                return Some(value);
            }
        }
    }

    fn atom(&mut self) -> Option<f64> {
        let mut neg = false;
        loop {
            if self.take(b'-') {
                neg = !neg;
            } else if !self.take(b'+') {
                break;
            }
        }
        let value = if self.take(b'(') {
            if self.depth >= 16 {
                return None;
            }
            self.depth += 1;
            let value = self.expr()?;
            self.depth -= 1;
            if !self.take(b')') {
                return None;
            }
            value
        } else {
            self.space();
            let start = self.pos;
            while self
                .input
                .get(self.pos)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                self.pos += 1;
            }
            std::str::from_utf8(&self.input[start..self.pos])
                .ok()?
                .parse::<f64>()
                .ok()?
        };
        Some(if neg { -value } else { value })
    }
}
