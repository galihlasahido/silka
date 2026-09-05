//! Team roster data — deterministic, no I/O.

/// One member of the team.
#[derive(Debug, Clone)]
pub struct Member {
    pub id: usize,
    pub name: String,
    pub role: String,
    pub bio: String,
}

/// The team lead's id — the one name in the header that opens a hover card.
pub const LEAD_ID: usize = 0;

/// The team as it stands before anyone is invited.
pub fn seed_members() -> Vec<Member> {
    [
        (
            0,
            "Ada Lovelace",
            "Engineering Lead",
            "Wrote the first algorithm for a machine that was never built.",
        ),
        (
            1,
            "Grace Hopper",
            "Principal Engineer",
            "Popularized the idea that code could be written in something \
             other than machine language.",
        ),
        (
            2,
            "Katherine Johnson",
            "Data Scientist",
            "Calculated the trajectories that got a spacecraft home.",
        ),
        (
            3,
            "Alan Turing",
            "Research Engineer",
            "Asked what it would take for a machine to think.",
        ),
        (
            4,
            "Margaret Hamilton",
            "Software Architect",
            "Coined the term software engineering while writing the code \
             that landed on the moon.",
        ),
        (
            5,
            "Radia Perlman",
            "Network Engineer",
            "Designed the protocol that keeps a network from routing \
             itself into a loop.",
        ),
    ]
    .into_iter()
    .map(|(id, name, role, bio)| Member {
        id,
        name: name.to_string(),
        role: role.to_string(),
        bio: bio.to_string(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lead_is_a_real_seed_member() {
        assert!(seed_members().iter().any(|m| m.id == LEAD_ID));
    }

    #[test]
    fn every_seed_member_has_a_name_a_role_and_a_bio() {
        for m in seed_members() {
            assert!(!m.name.is_empty());
            assert!(!m.role.is_empty());
            assert!(!m.bio.is_empty());
        }
    }
}
