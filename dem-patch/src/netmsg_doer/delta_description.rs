use crate::nom_helper::nom_fail;
use crate::types::{Delta, DeltaDecoder, DeltaDecoderS};

use super::*;

impl Doer for SvcDeltaDescription {
    fn id(&self) -> u8 {
        14
    }

    fn parse(i: &[u8], aux: AuxRefCell) -> Result<Self> {
        let (i, name) = null_string(i)?;
        let (i, total_fields) = le_u16(i)?;

        let clone = i;

        let mut aux = aux.borrow_mut();

        // Delta description is usually in LOADING section and first frame message.
        // It will detail the deltas being used and its index for correct decoding.
        // So this would be the only message that modifies the delta decode table.

        let mut br = BitReader::new(i);
        let default_table = crate::utils::get_initial_delta();
        let decoder = match aux.delta_decoders.get("delta_description_t\0") {
            Some(d) => d,
            None => default_table.get("delta_description_t\0").unwrap(),
        };
        let data: Vec<Delta> = (0..total_fields)
            .map(|_| {
                parse_delta(
                    decoder,
                    &mut br,
                )
            })
            .collect();

        // Some demos delta-compress each delta_description_t entry against the
        // one before it within this same message (name/bits/divisor/flags only
        // resent when changed), which this parser doesn't track a baseline for.
        // Bail with a normal parse error instead of unwrapping a missing field,
        // so a demo that hits this lands in the caller's existing
        // skip-unreadable-demo path rather than crashing the process.
        let mut decoder: DeltaDecoder = Vec::with_capacity(data.len());
        for entry in &data {
            let (Some(name), Some(bits), Some(divisor), Some(flags)) = (
                entry.get("name"),
                entry.get("bits"),
                entry.get("divisor"),
                entry.get("flags"),
            ) else {
                return nom_fail("delta_description_t entry missing name/bits/divisor/flags");
            };

            let (Ok(bits), Ok(divisor), Ok(flags)) = (
                bits.as_slice().try_into().map(u32::from_le_bytes),
                divisor.as_slice().try_into().map(f32::from_le_bytes),
                flags.as_slice().try_into().map(u32::from_le_bytes),
            ) else {
                return nom_fail("delta_description_t entry had a malformed bits/divisor/flags field");
            };

            decoder.push(DeltaDecoderS {
                name: name.to_owned(),
                bits,
                divisor,
                flags,
            });
        }

        // A read that ran past the end of this message's bytes means the
        // demo is malformed. Reject it here, where the caller's normal
        // parse-error path can skip the file -- the alternative is an
        // out-of-bounds index, and `panic = "abort"` makes that fatal to the
        // whole process rather than to this one demo. See #225.
        if br.is_bad_read() {
            return nom_fail("SvcDeltaDescription: read past the end of the message");
        }

        let range = br.get_consumed_bytes();
        let clone = &clone[..range];
        let (i, _) = take(range)(i)?;

        // mutate delta_decoders
        // `name` is a null-terminated string straight off disk; a corrupted
        // one is not a reason to abort the process. #225.
        aux.delta_decoders
            .insert(String::from_utf8_lossy(name).into_owned(), decoder.clone());

        Ok((
            i,
            Self {
                name: name.to_vec(),
                total_fields,
                fields: decoder,
                clone: clone.to_vec(),
            },
        ))
    }

    fn write(&self, _: AuxRefCell) -> ByteVec {
        let mut writer = ByteWriter::new();

        writer.append_u8(self.id());

        writer.append_u8_slice(&self.name);
        writer.append_u16(self.total_fields);

        // This is intentionally done like this because I don't think anyone
        // would try to modify delta description.
        writer.append_u8_slice(&self.clone);

        writer.data
    }
}
