use crate::nom_helper::nom_fail;
use crate::types::ClientDataWeaponData;

use super::*;

impl Doer for SvcClientData {
    fn id(&self) -> u8 {
        15
    }

    fn parse(i: &[u8], aux: AuxRefCell) -> Result<Self> {
        let aux = aux.borrow();

        if aux.is_hltv {
            return Ok((
                i,
                Self {
                    has_delta_update_mask: false,
                    delta_update_mask: None,
                    client_data: Default::default(),
                    weapon_data: None,
                },
            ));
        }

        let mut br = BitReader::new(i);

        let has_delta_update_mask = br.read_1_bit();
        let delta_update_mask = if has_delta_update_mask {
            Some(br.read_n_bit(8).to_owned())
        } else {
            None
        };

        let client_decoder = match aux.delta_decoders.get("clientdata_t\0") {
            Some(d) => d,
            None => return nom_fail("missing clientdata_t decoder"),
        };
        let client_data = parse_delta(client_decoder, &mut br);

        // This is a vector unlike THE docs.
        let mut weapon_data: Vec<ClientDataWeaponData> = vec![];
        let weapon_decoder = match aux.delta_decoders.get("weapon_data_t\0") {
            Some(d) => d,
            None => return nom_fail("missing weapon_data_t decoder"),
        };
        while br.read_1_bit() {
            let weapon_index = br.read_n_bit(6).to_owned();
            let delta = parse_delta(weapon_decoder, &mut br);

            weapon_data.push(ClientDataWeaponData {
                weapon_index,
                weapon_data: delta,
            });
        }

        let weapon_data = if weapon_data.is_empty() {
            None
        } else {
            Some(weapon_data)
        };

        // Remember to write the last "false" bit.

        // A read that ran past the end of this message's bytes means the
        // demo is malformed. Reject it here, where the caller's normal
        // parse-error path can skip the file -- the alternative is an
        // out-of-bounds index, and `panic = "abort"` makes that fatal to the
        // whole process rather than to this one demo. See #225.
        if br.is_bad_read() {
            return nom_fail("SvcClientData: read past the end of the message");
        }

        let range = br.get_consumed_bytes();
        let (i, _) = take(range)(i)?;

        Ok((
            i,
            Self {
                has_delta_update_mask,
                delta_update_mask,
                client_data,
                weapon_data,
            },
        ))
    }

    fn write(&self, aux: AuxRefCell) -> ByteVec {
        let aux = aux.borrow();

        let mut writer = ByteWriter::new();

        writer.append_u8(self.id());

        if aux.is_hltv {
            return writer.data;
        }

        let mut bw = BitWriter::new();

        bw.append_bit(self.has_delta_update_mask);

        if self.has_delta_update_mask {
            bw.append_vec(self.delta_update_mask.as_ref().unwrap());
        }

        write_delta(
            &self.client_data,
            aux.delta_decoders.get("clientdata_t\0").unwrap(),
            &mut bw,
        );

        if let Some(weapon_data) = &self.weapon_data {
            for data in weapon_data {
                bw.append_bit(true);
                bw.append_vec(&data.weapon_index);
                write_delta(
                    &data.weapon_data,
                    aux.delta_decoders.get("weapon_data_t\0").unwrap(),
                    &mut bw,
                );
            }
        }

        // false bit for weapon data
        bw.append_bit(false);

        writer.append_u8_slice(&bw.get_u8_vec());

        writer.data
    }
}
