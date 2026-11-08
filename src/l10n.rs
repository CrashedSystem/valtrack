//! Korean display names, taken from Riot's own ko-KR strings (via the
//! valorant-api snapshot) so the overlay matches what the KR client shows.
//!
//! Tables are keyed by agent UUID and by the English map name that
//! [`crate::rank::map_name`] already resolves, which keeps them independent of
//! the generated `assets.rs` file.

static AGENTS_KO: &[(&str, &str)] = &[
    ("e370fa57-4757-3604-3648-499e1f642d3f", "게코"),
    ("bb2a4828-46eb-8cd1-e765-15848195d751", "네온"),
    ("cc8b64c8-4b25-4ff9-6e7f-37b4da43d235", "데드록"),
    ("a3bfb853-43b2-7238-a4f1-ad90e9e46bcc", "레이나"),
    ("f94c3b30-42be-e959-889c-5aa313dba261", "레이즈"),
    ("7c8a4701-4de6-9355-b254-e09bc2a34b72", "믹스"),
    ("efba5359-4016-a1e5-7626-b1ae76895940", "바이스"),
    ("707eab51-4836-f488-046a-cda6bf494859", "바이퍼"),
    ("5f8d3a7f-467b-97f3-062c-13acf203c006", "브리치"),
    ("9f0d8ba9-4140-b941-57d3-a7ad57c6b417", "브림스톤"),
    ("92eeef5d-43b5-1d4a-8d03-b3927a09034b", "비토"),
    ("117ed9e3-49f3-6512-3ccf-0cada7e3823b", "사이퍼"),
    ("569fdd95-4d10-43ab-ca70-79becc718b46", "세이지"),
    ("320b2a48-4d9b-a075-30f1-1f93a9b638fa", "소바"),
    ("6f2a04ca-43e0-be17-7f36-b3908627744d", "스카이"),
    ("41fb69c1-4189-7b37-f117-bcaf1e96f1bf", "아스트라"),
    ("0e38b510-41a8-5780-5e8f-568b2a4f2d6c", "아이소"),
    ("8e253930-4c05-31dd-1b6c-968525494517", "오멘"),
    ("7f94d92c-4234-0a36-9646-3a87eb8b5c89", "요루"),
    ("df1cb487-4902-002e-5c17-d28e83e78588", "웨이레이"),
    ("add6443a-41bd-e414-f6ad-e58d267f4e95", "제트"),
    ("22697a3d-45bf-8dd7-4fec-84a9e28c69d7", "체임버"),
    ("601dbbe7-43ce-be57-2a40-4abd24953621", "케이/오"),
    // Same agent, Ability Draft (건틀릿) variant: not a playable character, but
    // the game can still report this UUID as a roster CharacterID.
    ("773f0c78-4486-752b-68ef-4585d7f4b848", "케이/오"),
    ("1dbf2edd-4729-0984-3115-daa5eed44993", "클로브"),
    ("1e58de9c-4950-5125-93e9-a0aee9f98746", "킬조이"),
    ("b444168c-4e35-8076-db47-ef9bf368f384", "테호"),
    ("dade69b4-4f5a-8528-247b-219e5a1facd6", "페이드"),
    ("eb93336a-449b-9c1b-0a54-a891f7921d69", "피닉스"),
    ("95b78ed7-4637-86d9-7e41-71ba8c293152", "하버"),
    ("ddd5a15c-4fbe-42fa-3789-11a55ca5ee04", "허시"),
];

static MAPS_KO: &[(&str, &str)] = &[
    ("Gauntlet", "건틀릿"),
    ("Ascent", "어센트"),
    ("Split", "스플릿"),
    ("Fracture", "프랙처"),
    ("Bind", "바인드"),
    ("Breeze", "브리즈"),
    ("District", "디스트릭트"),
    ("Kasbah", "카즈바"),
    ("Drift", "드리프트"),
    ("Glitch", "글리치"),
    ("Piazza", "피아자"),
    ("Abyss", "어비스"),
    ("Lotus", "로터스"),
    ("Sunset", "선셋"),
    ("Basic Training", "기초 훈련"),
    ("Pearl", "펄"),
    ("Summit", "서밋"),
    ("Icebox", "아이스박스"),
    ("The Range", "사격장"),
    ("Corrode", "코로드"),
    ("Skirmish A", "난투 A"),
    ("Skirmish B", "난투 B"),
    ("Skirmish C", "난투 C"),
    ("Skirmish D", "난투 D"),
    ("Skirmish E", "난투 E"),
    ("Haven", "헤이븐"),
];

pub fn agent_ko(agent_id: &str) -> Option<&'static str> {
    AGENTS_KO
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(agent_id))
        .map(|(_, name)| *name)
}

pub fn map_ko(name_en: &str) -> Option<&'static str> {
    MAPS_KO
        .iter()
        .find(|(en, _)| *en == name_en)
        .map(|(_, name)| *name)
}
