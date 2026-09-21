use crate::nom_helper::nom_fail;
use super::*;

impl Doer for SvcEventReliable {
    fn id(&self) -> u8 {
        21
    }

    fn parse(i: &[u8], aux: AuxRefCell) -> Result<Self> {
        let aux = aux.borrow();

        let mut br = BitReader::new(i);

        let event_index = br.read_n_bit(10).to_owned();
        let event_decoder = match aux.delta_decoders.get("event_t\0") {
            Some(d) => d,
            None => return nom_fail("missing event_t decoder"),
        };
        let event_args = parse_delta(event_decoder, &mut br);
        let has_fire_time = br.read_1_bit();
        let fire_time = if has_fire_time {
            Some(br.read_n_bit(16).to_owned())
        } else {
            None
        };

        // A read that ran past the end of this message's bytes means the
        // demo is malformed. Reject it here, where the caller's normal
        // parse-error path can skip the file -- the alternative is an
        // out-of-bounds index, and `panic = "abort"` makes that fatal to the
        // whole process rather than to this one demo. See #225.
        if br.is_bad_read() {
            return nom_fail("SvcEventReliable: read past the end of the message");
        }

        let range = br.get_consumed_bytes();
        let (i, _) = take(range)(i)?;

        Ok((
            i,
            Self {
                event_index,
                event_args,
                has_fire_time,
                fire_time,
            },
        ))
    }

    fn write(&self, aux: AuxRefCell) -> ByteVec {
        let aux = aux.borrow();

        let mut writer = ByteWriter::new();
        let mut bw = BitWriter::new();

        writer.append_u8(self.id());

        bw.append_vec(&self.event_index);
        write_delta(
            &self.event_args,
            aux.delta_decoders.get("event_t\0").unwrap(),
            &mut bw,
        );

        bw.append_bit(self.has_fire_time);
        if self.has_fire_time {
            bw.append_vec(self.fire_time.as_ref().unwrap());
        }

        writer.append_u8_slice(&bw.get_u8_vec());

        writer.data
    }
}
