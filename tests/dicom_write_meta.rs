//! dicomwrite metadata: info-struct copy mode, VR/Endian and WritePrivate.
//! Synthetic fixtures only; pydicom cross-check in tool/refs/dicom/write_meta_check.py.
use matrust::Engine;

/// Output syntaxes: explicit LE, implicit LE, explicit BE.
const SYNTAXES: [&str; 3] = [
    "1.2.840.10008.1.2.1",
    "1.2.840.10008.1.2",
    "1.2.840.10008.1.2.2",
];
/// pydicom-written sources (tool/refs/dicom/info.py): PN, nested sequences,
/// VM>1 text and numbers, private tags and a sequence after PixelData.
const SOURCES: [&str; 3] = [
    "explicit_le_defined",
    "implicit_le_defined",
    "explicit_be_defined",
];
const SC: &str = "'SOPClassUID','1.2.840.10008.5.1.4.1.1.7','SOPInstanceUID','2.25.1234'";
/// Hand-built struct; tool/refs/dicom/write_meta_check.py holds the same literals.
const MANUAL: &str = r"M=struct('SOPClassUID','1.2.840.10008.5.1.4.1.1.7','SOPInstanceUID','2.25.1234','PhotometricInterpretation','MONOCHROME1','PatientName','Yamada^Tarou=山田^太郎=やまだ^たろう','ReferringPhysicianName',struct('FamilyName','Müller','GivenName','Zoë','MiddleName','','NamePrefix','Dr','NameSuffix',''),'OtherPatientNames','Alpha^A\Beta^B=β','ImageType','DERIVED\SECONDARY','PixelSpacing',[0.5;0.25],'ImagePositionPatient','-10.5\2.25\0','RescaleSlope',1e-7/3,'AcquisitionNumber',7,'ReferencedFrameNumber',[1;2;300],'FrameIncrementPointer',uint32([1577059;1577061]),'SelectorUSValue',uint16([1;65535;42]),'SelectorSSValue',int16([-123;321]),'SelectorSLValue',int32([-2147483648;2147483647]),'SelectorULValue',uint32([0;4294967295]),'SelectorSVValue',int64([-5;7]),'SelectorUVValue',uint64([0;9]),'SelectorFLValue',single([-1.25;2.5]),'SelectorFDValue',[-123.125;1e100],'StudyDate','20000102','StudyTime','030405.125','AcquisitionDateTime','20000102030405.125000+0200','PatientAge','042Y','SmallestImagePixelValue',5,'EncapsulatedDocument',uint8([1;2;3]),'RedPaletteColorLookupTableData',uint16([0;258;65535]),'OverlayRows_2',4,'Rows',999,'Columns',999,'BitsStored',3,'Filename','ignored.dcm','FileModDate','ignored','FileSize',1,'Format','DICOM','FormatVersion',3,'Width',9,'Height',9,'BitDepth',1,'ColorType','grayscale','StartOfPixelData',5,'FileMetaInformationGroupLength',1,'TransferSyntaxUID','1.2.840.10008.1.2.4.50','Private_0011_10xx_Creator','DROPPED'); M.ReferencedStudySequence=struct('Item_1',struct('ReferencedSOPClassUID','1.2.840.10008.3.1.2.3.1','ReferencedSOPInstanceUID','2.25.99','PurposeOfReferenceCodeSequence',struct('Item_1',struct('CodeValue','121322','CodeMeaning','Source café'))),'Item_2',struct()); M.DigitalSignaturesSequence=struct('Item_1',struct());";
const PRIVATE: &str = "I.Private_0011_10xx_Creator='SYNTHETIC_PRIVATE'; I.Private_0011_1010=uint8([254;255;221;224;0;0;0;0]);";

fn fixture(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
        .to_string_lossy()
        .replace('\'', "''")
}
fn folder(path: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(path).unwrap();
    e
}
fn logical(e: &Engine, name: &str) -> bool {
    e.array(name).unwrap().scalar(0, 0).unwrap() == matrust::Scalar::Logical(true)
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap() {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        a => panic!("expected character, got {}", a.class()),
    }
}
/// Every listed expression must be true.
fn all(e: &mut Engine, checks: &[&str]) {
    for check in checks {
        e.execute(&format!("ok=({check});"))
            .unwrap_or_else(|error| panic!("{check}: {error}"));
        assert!(logical(e, "ok"), "{check}");
    }
}
fn entries(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}
/// The 21 files inspected by write_meta_check.py; byte-stable (no generated values).
fn generate(dir: &std::path::Path) {
    let mut e = folder(dir);
    for source in SOURCES {
        let path = fixture(&format!("{source}.dcm"));
        e.execute(&format!("I=dicominfo('{path}'); X=dicomread('{path}');"))
            .unwrap();
        for (k, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!(
                "dicomwrite(X,'copy_{source}_{k}.dcm',I,'CreateMode','copy','TransferSyntax','{syntax}');"
            ))
            .unwrap();
        }
        e.execute(PRIVATE).unwrap();
        for (k, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!(
                "dicomwrite(X,'private_{source}_{k}.dcm',I,'CreateMode','copy','WritePrivate',true,'TransferSyntax','{syntax}');"
            ))
            .unwrap();
        }
    }
    e.execute(MANUAL).unwrap();
    for (k, syntax) in SYNTAXES.iter().enumerate() {
        e.execute(&format!(
            "dicomwrite(uint8([7 250]),'manual_{k}.dcm',M,'CreateMode','copy','TransferSyntax','{syntax}');"
        ))
        .unwrap();
    }
}

/// 7a11 baseline: on HEAD 757ee4b8 this fails at the option guard
/// ("unsupported option 'CreateMode'; only TransferSyntax is supported").
#[test]
fn copy_mode_public_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    let source = fixture("explicit_le_defined.dcm");
    e.execute(&format!(
        "I=dicominfo('{source}'); X=dicomread('{source}'); dicomwrite(X,'copy.dcm',I,'CreateMode','copy'); J=dicominfo('copy.dcm'); Y=dicomread('copy.dcm');"
    ))
    .unwrap();
    e.execute("ok=isequal(X,Y) && isequal(J.PatientName,I.PatientName) && isequal(J.ReferencedStudySequence,I.ReferencedStudySequence) && strcmp(J.SOPInstanceUID,I.SOPInstanceUID) && isequal(J.PixelSpacing,I.PixelSpacing);").unwrap();
    assert!(logical(&e, "ok"));
}

#[test]
fn three_sources_round_trip_under_each_output_syntax() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path());
    let mut e = folder(dir.path());
    // Regenerated file meta, X-derived attributes and dicominfo bookkeeping.
    e.execute("skip={'Filename','FileSize','FileMetaInformationGroupLength','TransferSyntaxUID','ImplementationClassUID','ImplementationVersionName','BitsStored','HighBit','BitDepth','PlanarConfiguration','NumberOfFrames'};").unwrap();
    for source in SOURCES {
        let path = fixture(&format!("{source}.dcm"));
        e.execute(&format!("I=dicominfo('{path}'); X=dicomread('{path}');"))
            .unwrap();
        for (k, syntax) in SYNTAXES.iter().enumerate() {
            for kind in ["copy", "private"] {
                let name = format!("{kind}_{source}_{k}.dcm");
                e.execute(&format!("J=dicominfo('{name}'); Y=dicomread('{name}'); names=fieldnames(I); bad=''; n=0; for q=1:numel(names), f=names{{q}}; if any(strcmp(f,skip)), continue; end; n=n+1; if ~isfield(J,f) || ~isequal(I.(f),J.(f)), bad=[bad ' ' f]; end; end; extra=numel(fieldnames(J))-numel(names);")).unwrap();
                assert_eq!(text_or_empty(&e, "bad"), "", "{name}");
                all(
                    &mut e,
                    &[
                        // 41 of the source's 52 dicominfo fields are compared here.
                        "n==41",
                        // Only PlanarConfiguration and NumberOfFrames disappear.
                        "extra==-2",
                        "isequal(X,Y) && isa(Y,'uint16')",
                        &format!("strcmp(J.TransferSyntaxUID,'{syntax}')"),
                        // The source stores 12 bits; X is uint16 and wins.
                        "I.BitsStored==12 && J.BitsStored==16 && J.HighBit==15 && J.BitDepth==16",
                        "~isfield(J,'PlanarConfiguration') && ~isfield(J,'NumberOfFrames')",
                        "strcmp(J.MediaStorageSOPInstanceUID,I.SOPInstanceUID) && strcmp(J.MediaStorageSOPClassUID,I.SOPClassUID)",
                        "strcmp(J.PatientName.GivenName,'Zoë') && strcmp(J.ReferencedStudySequence.Item_1.ReferencedImageSequence.Item_1.PatientName.FamilyName,'Synthétique')",
                        "numel(fieldnames(J.ReferencedStudySequence.Item_2))==0 && isfield(J,'DigitalSignaturesSequence')",
                    ],
                );
            }
        }
    }
}
fn text_or_empty(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap() {
        a if a.is_empty() => String::new(),
        _ => text(e, name),
    }
}

#[test]
fn committed_outputs_are_reproduced_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path());
    let committed =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom/write_meta");
    assert_eq!(entries(dir.path()), 21);
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            std::fs::read(entry.path()).unwrap(),
            std::fs::read(committed.join(entry.file_name())).unwrap(),
            "{:?}",
            entry.file_name()
        );
    }
}

#[test]
fn value_forms_person_names_sequences_multiplicity_and_tags() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path());
    let mut e = folder(dir.path());
    for (k, syntax) in SYNTAXES.iter().enumerate() {
        e.execute(&format!(
            "J=dicominfo('manual_{k}.dcm'); Y=dicomread('manual_{k}.dcm');"
        ))
        .unwrap();
        all(
            &mut e,
            &[
                "isequal(Y,uint8([7 250]))",
                // PN given as char: three component groups.
                "strcmp(J.PatientName.FamilyName,'Yamada') && strcmp(J.PatientName.GivenName,'Tarou') && strcmp(J.PatientName.Ideographic.FamilyName,'山田') && strcmp(J.PatientName.Phonetic.GivenName,'たろう')",
                // PN given as the dicominfo struct form; trailing empty components are trimmed.
                "strcmp(J.ReferringPhysicianName.FamilyName,'Müller') && strcmp(J.ReferringPhysicianName.GivenName,'Zoë') && strcmp(J.ReferringPhysicianName.NamePrefix,'Dr') && isempty(J.ReferringPhysicianName.MiddleName)",
                "numel(J.OtherPatientNames)==2 && strcmp(J.OtherPatientNames(2).FamilyName,'Beta') && strcmp(J.OtherPatientNames(2).Ideographic.FamilyName,'β')",
                "strcmp(J.SpecificCharacterSet,'ISO_IR 192')",
                // VM>1 text and numbers.
                r"strcmp(J.ImageType,'DERIVED\SECONDARY')",
                "isequal(J.PixelSpacing,[0.5;0.25]) && isequal(J.ImagePositionPatient,[-10.5;2.25;0])",
                "abs(J.RescaleSlope-1e-7/3)<1e-17 && J.AcquisitionNumber==7 && isequal(J.ReferencedFrameNumber,[1;2;300])",
                "isequal(J.SelectorUSValue,uint16([1;65535;42])) && isequal(J.SelectorSSValue,int16([-123;321]))",
                "isequal(J.SelectorSLValue,int32([-2147483648;2147483647])) && isequal(J.SelectorULValue,uint32([0;4294967295]))",
                "isequal(J.SelectorSVValue,int64([-5;7])) && isequal(J.SelectorUVValue,uint64([0;9]))",
                "isequal(J.SelectorFLValue,single([-1.25;2.5])) && isequal(J.SelectorFDValue,[-123.125;1e100])",
                // AT keeps (group,element) pairs.
                "isequal(J.FrameIncrementPointer,uint32([1577059;1577061]))",
                "strcmp(J.StudyDate,'20000102') && strcmp(J.StudyTime,'030405.125') && strcmp(J.AcquisitionDateTime,'20000102030405.125000+0200') && strcmp(J.PatientAge,'042Y')",
                // US/SS follows PixelRepresentation of the written uint8 image.
                "isa(J.SmallestImagePixelValue,'uint16') && J.SmallestImagePixelValue==5",
                // OB keeps its odd byte count plus the NUL pad; OW keeps 16-bit words.
                "isequal(J.EncapsulatedDocument,uint8([1;2;3;0])) && isequal(J.RedPaletteColorLookupTableData,uint16([0;258;65535]))",
                "J.OverlayRows_2==4",
                // Nested sequence, empty item and a tag above PixelData.
                "strcmp(J.ReferencedStudySequence.Item_1.PurposeOfReferenceCodeSequence.Item_1.CodeMeaning,'Source café') && strcmp(J.ReferencedStudySequence.Item_1.ReferencedSOPInstanceUID,'2.25.99')",
                "numel(fieldnames(J.ReferencedStudySequence))==2 && numel(fieldnames(J.ReferencedStudySequence.Item_2))==0 && numel(fieldnames(J.DigitalSignaturesSequence))==1",
                // X wins over Rows=999, Columns=999, BitsStored=3; MONOCHROME1 is kept.
                "J.Rows==1 && J.Columns==2 && J.BitsStored==8 && J.HighBit==7 && J.BitsAllocated==8 && J.PixelRepresentation==0 && J.SamplesPerPixel==1",
                "strcmp(J.PhotometricInterpretation,'MONOCHROME1')",
                // Bookkeeping fields describe the new file, not the struct's values.
                "J.FileSize>1000 && J.Width==2 && J.Height==1 && J.BitDepth==8 && ~isfield(J,'StartOfPixelData') && ~isfield(J,'FormatVersion')",
                &format!("strcmp(J.TransferSyntaxUID,'{syntax}')"),
                // Copy mode generates nothing: no Create-mode defaults or UIDs.
                "~isfield(J,'StudyInstanceUID') && ~isfield(J,'Modality') && ~isfield(J,'PatientID') && strcmp(J.SOPInstanceUID,'2.25.1234')",
            ],
        );
        // The struct form dicominfo produced (VM 2, Ideographic) writes the same file again.
        e.execute(&format!(
            "dicomwrite(Y,'again_{k}.dcm',J,'CreateMode','copy','TransferSyntax','{syntax}');"
        ))
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join(format!("again_{k}.dcm"))).unwrap(),
            std::fs::read(dir.path().join(format!("manual_{k}.dcm"))).unwrap()
        );
        // The private creator in the struct is dropped by default.
        let bytes = std::fs::read(dir.path().join(format!("manual_{k}.dcm"))).unwrap();
        assert!(!contains(&bytes, b"DROPPED"));
    }
    // Literal bytes: AT pairs and a defined-length sequence in each syntax.
    let le = std::fs::read(dir.path().join("manual_0.dcm")).unwrap();
    assert!(contains(
        &le,
        &[
            0x28, 0, 9, 0, b'A', b'T', 8, 0, 0x18, 0, 0x63, 0x10, 0x18, 0, 0x65, 0x10
        ]
    ));
    assert!(contains(
        &le,
        &[
            0xfa, 0xff, 0xfa, 0xff, b'S', b'Q', 0, 0, 8, 0, 0, 0, 0xfe, 0xff, 0, 0xe0, 0, 0, 0, 0
        ]
    ));
    let implicit = std::fs::read(dir.path().join("manual_1.dcm")).unwrap();
    assert!(contains(
        &implicit,
        &[
            0x28, 0, 9, 0, 8, 0, 0, 0, 0x18, 0, 0x63, 0x10, 0x18, 0, 0x65, 0x10
        ]
    ));
    assert!(implicit.ends_with(&[
        0xfa, 0xff, 0xfa, 0xff, 8, 0, 0, 0, 0xfe, 0xff, 0, 0xe0, 0, 0, 0, 0
    ]));
    let be = std::fs::read(dir.path().join("manual_2.dcm")).unwrap();
    assert!(contains(
        &be,
        &[
            0, 0x28, 0, 9, b'A', b'T', 0, 8, 0, 0x18, 0x10, 0x63, 0, 0x18, 0x10, 0x65
        ]
    ));
    assert!(be.ends_with(&[
        0xff, 0xfa, 0xff, 0xfa, b'S', b'Q', 0, 0, 0, 0, 0, 8, 0xff, 0xfe, 0xe0, 0, 0, 0, 0, 0
    ]));
}

#[test]
fn write_private_false_true_and_custom_dictionary_vr() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path());
    let creator = b"SYNTHETIC_PRIVATE ";
    let data = [254, 255, 221, 224, 0, 0, 0, 0];
    for source in SOURCES {
        for k in 0..3 {
            let dropped = std::fs::read(dir.path().join(format!("copy_{source}_{k}.dcm"))).unwrap();
            assert!(!contains(&dropped, b"SYNTHETIC_PRIVATE"), "{source} {k}");
            let kept = std::fs::read(dir.path().join(format!("private_{source}_{k}.dcm"))).unwrap();
            let (lo, un): (Vec<u8>, Vec<u8>) = match k {
                0 => (
                    [&[0x11, 0, 0x10, 0, b'L', b'O', 18, 0][..], creator].concat(),
                    [
                        &[0x11, 0, 0x10, 0x10, b'U', b'N', 0, 0, 8, 0, 0, 0][..],
                        &data,
                    ]
                    .concat(),
                ),
                1 => (
                    [&[0x11, 0, 0x10, 0, 18, 0, 0, 0][..], creator].concat(),
                    [&[0x11, 0, 0x10, 0x10, 8, 0, 0, 0][..], &data].concat(),
                ),
                _ => (
                    [&[0, 0x11, 0, 0x10, b'L', b'O', 0, 18][..], creator].concat(),
                    [
                        &[0, 0x11, 0x10, 0x10, b'U', b'N', 0, 0, 0, 0, 0, 8][..],
                        &data,
                    ]
                    .concat(),
                ),
            };
            assert!(contains(&kept, &lo) && contains(&kept, &un), "{source} {k}");
            assert_eq!(kept.len(), dropped.len() + lo.len() + un.len());
        }
    }
    let mut e = folder(dir.path());
    // Private names need a private group (odd, not 0001-0007 or FFFF).
    for bad in [
        "Private_0008_0010",
        "Private_0007_1000",
        "Private_FFFF_1000",
    ] {
        let error = e
            .execute(&format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'{bad}',uint8(1)),'CreateMode','copy','WritePrivate',true)"
            ))
            .unwrap_err();
        assert!(error.contains(bad) && error.contains("private"), "{error}");
    }
    // A private group length is dropped; the documented Private_0009_0006 form is written as UN.
    e.execute(&format!("dicomwrite(uint8(1),'plain.dcm',struct({SC}),'CreateMode','copy'); dicomwrite(uint8(1),'length.dcm',struct({SC},'Private_0009_0000',uint32(4),'Private_0009_0006',uint8([5;6])),'CreateMode','copy','WritePrivate',true);")).unwrap();
    let plain = std::fs::read(dir.path().join("plain.dcm")).unwrap();
    let length = std::fs::read(dir.path().join("length.dcm")).unwrap();
    let element = [9, 0, 6, 0, b'U', b'N', 0, 0, 2, 0, 0, 0, 5, 6];
    assert!(contains(&length, &element) && length.len() == plain.len() + element.len());
    // A custom session dictionary is read only for the VR of private tags.
    let custom = fixture("custom-dict.txt");
    e.execute(&format!("dicomdict('set','{custom}'); S=struct({SC},'PrivateCount',513,'PrivateLabel','PRIVATE','Private_0011_10xx_Creator','ACME','Private_0011_1003',uint8([9;8]));")).unwrap();
    for refused in [
        "dicomwrite(uint8([1 2 3;4 5 6]),'bad.dcm',S,'CreateMode','copy')",
        "dicomwrite(uint8([1 2 3;4 5 6]),'bad.dcm',S,'CreateMode','copy','WritePrivate',false)",
    ] {
        let error = e.execute(refused).unwrap_err();
        assert!(
            error.contains("custom dictionary") && error.contains("not supported yet"),
            "{error}"
        );
    }
    let error = e
        .execute(&format!("dicomwrite(uint8(1),'bad.dcm',S,'CreateMode','copy','WritePrivate',true,'Dictionary','{custom}')"))
        .unwrap_err();
    assert!(
        error.contains("Dictionary") && error.contains("not supported yet"),
        "{error}"
    );
    // Custom keywords of standard tags are not writer names.
    let error = e
        .execute(&format!("dicomwrite(uint8(1),'bad.dcm',struct({SC},'CustomOverlayRows_0',4),'CreateMode','copy','WritePrivate',true)"))
        .unwrap_err();
    assert!(error.contains("CustomOverlayRows_0"), "{error}");
    assert!(!dir.path().join("bad.dcm").exists());
    e.execute("dicomwrite(uint8([1 2 3;4 5 6]),'custom.dcm',S,'CreateMode','copy','WritePrivate',true); J=dicominfo('custom.dcm'); Y=dicomread('custom.dcm');").unwrap();
    all(
        &mut e,
        &[
            "isa(J.PrivateCount,'uint16') && J.PrivateCount==513 && strcmp(J.PrivateLabel,'PRIVATE')",
            "isequal(Y,uint8([1 2 3;4 5 6]))",
        ],
    );
    let bytes = std::fs::read(dir.path().join("custom.dcm")).unwrap();
    for needle in [
        &[0x11, 0, 0x10, 0, b'L', b'O', 4, 0, b'A', b'C', b'M', b'E'][..],
        &[0x11, 0, 1, 0x10, b'U', b'S', 2, 0, 1, 2],
        &[
            0x11, 0, 2, 0x10, b'L', b'O', 8, 0, b'P', b'R', b'I', b'V', b'A', b'T', b'E', b' ',
        ],
        &[0x11, 0, 3, 0x10, b'U', b'N', 0, 0, 2, 0, 0, 0, 9, 8],
    ] {
        assert!(contains(&bytes, needle), "{needle:?}");
    }
    e.execute("dicomdict('factory');").unwrap();
}

#[test]
fn vr_endian_and_transfer_syntax_combinations_and_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    let (explicit, implicit, big) = (SYNTAXES[0], SYNTAXES[1], SYNTAXES[2]);
    for (options, expected) in [
        ("", explicit),
        (",'VR','explicit'", explicit),
        (",'VR','implicit'", implicit),
        (",'Endian','ieee-le'", explicit),
        (",'Endian','ieee-be'", big),
        (",'VR','explicit','Endian','ieee-be'", big),
        (",'vr','IMPLICIT','endian','IEEE-LE'", implicit),
        (",\"VR\",\"implicit\"", implicit),
        // TransferSyntax overrides VR and Endian, whatever their order or agreement.
        (
            ",'TransferSyntax','1.2.840.10008.1.2','VR','explicit','Endian','ieee-be'",
            implicit,
        ),
        (
            ",'VR','implicit','Endian','ieee-be','TransferSyntax','1.2.840.10008.1.2.2'",
            big,
        ),
        (
            ",'Endian','ieee-be','TransferSyntax','1.2.840.10008.1.2.1'",
            explicit,
        ),
        // Documented defaults of options outside this slice are no-ops.
        (
            ",'CompressionMode','None','UseMetadataBitDepths',false,'MultiframeSingleFile',true",
            explicit,
        ),
        (
            ",'ObjectType','Secondary Capture Image Storage','SOPClassUID','1.2.840.10008.5.1.4.1.1.7','CreateMode','Create','WritePrivate',false",
            explicit,
        ),
        (",'WritePrivate',1,'Endian','ieee-be'", big),
    ] {
        e.execute(&format!(
            "dicomwrite(int16([-2 3;4 -5]),'ok.dcm'{options}); J=dicominfo('ok.dcm'); Y=dicomread('ok.dcm');"
        ))
        .unwrap_or_else(|error| panic!("{options}: {error}"));
        all(
            &mut e,
            &[
                &format!("strcmp(J.TransferSyntaxUID,'{expected}')"),
                "isequal(Y,int16([-2 3;4 -5]))",
            ],
        );
        std::fs::remove_file(dir.path().join("ok.dcm")).unwrap();
    }
    for (options, expected) in [
        (
            ",'VR','implicit','Endian','ieee-be'",
            "not a DICOM transfer syntax",
        ),
        (
            ",'Endian','ieee-be','VR','implicit'",
            "not a DICOM transfer syntax",
        ),
        (",'VR','bogus'", "VR must be"),
        (",'VR',1", "VR"),
        (",'Endian','vax'", "Endian must be"),
        (
            ",'Endian','ieee-be','TransferSyntax','1.2.840.10008.1.2.1','VR','bogus'",
            "VR must be",
        ),
        (",'VR','explicit','vr','explicit'", "duplicate"),
        (",'Endian','ieee-le','Endian','ieee-be'", "duplicate"),
        (",'CreateMode','copy','CreateMode','copy'", "duplicate"),
        (",'CreateMode','bogus'", "CreateMode must be"),
        (",'CreateMode','copy'", "needs a metadata struct"),
        (",'WritePrivate','yes'", "WritePrivate must be"),
        (",'WritePrivate',2", "WritePrivate must be"),
        (
            ",'CompressionMode','JPEG lossless'",
            "CompressionMode 'JPEG lossless' is not supported yet",
        ),
        (
            ",'CompressionMode','RLE','TransferSyntax','1.2.840.10008.1.2.1'",
            "CompressionMode 'RLE' is not supported yet",
        ),
        (
            ",'UseMetadataBitDepths',true",
            "UseMetadataBitDepths true is not supported yet",
        ),
        (
            ",'MultiframeSingleFile',false",
            "MultiframeSingleFile false is not supported yet",
        ),
        (
            ",'ObjectType','CT Image Storage'",
            "ObjectType 'CT Image Storage' is not supported yet",
        ),
        (
            ",'SOPClassUID','1.2.840.10008.5.1.4.1.1.2'",
            "SOPClassUID '1.2.840.10008.5.1.4.1.1.2' is not supported yet",
        ),
        (
            ",'Dictionary','d.txt'",
            "Dictionary option is not supported yet",
        ),
        (",'Bogus',1", "unsupported option 'Bogus'"),
        (",'VR'", "name/value pairs"),
    ] {
        let error = e
            .execute(&format!(
                "dicomwrite(int16([-2 3;4 -5]),'bad.dcm'{options})"
            ))
            .unwrap_err();
        assert!(error.contains(expected), "{options}: {error}");
        assert_eq!(entries(dir.path()), 0, "{options}");
    }
}

#[test]
fn derived_fields_win_bookkeeping_ignored_and_field_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    // Copy mode: every contradicting pixel attribute is replaced by X's value.
    e.execute(&format!("S=struct({SC},'Rows',999,'Columns',999,'BitsAllocated',32,'BitsStored',3,'HighBit',2,'PixelRepresentation',1,'SamplesPerPixel',3,'PhotometricInterpretation','RGB','NumberOfFrames',9,'PlanarConfiguration',1,'PixelData',uint8(5),'Filename','x.dcm','FileSize',5,'Width',1,'Height',1,'BitDepth',1,'ColorType','truecolor','StartOfPixelData',1,'Format','x','FormatVersion',3,'FileModDate','x'); X=uint16([1 2 3;4 5 6]); dicomwrite(X,'win.dcm',S,'CreateMode','copy'); J=dicominfo('win.dcm'); Y=dicomread('win.dcm');")).unwrap();
    all(
        &mut e,
        &[
            "isequal(X,Y)",
            "J.Rows==2 && J.Columns==3 && J.BitsAllocated==16 && J.BitsStored==16 && J.HighBit==15 && J.PixelRepresentation==0 && J.SamplesPerPixel==1",
            "strcmp(J.PhotometricInterpretation,'MONOCHROME2') && ~isfield(J,'NumberOfFrames') && ~isfield(J,'PlanarConfiguration')",
            "J.Width==3 && J.Height==2 && J.BitDepth==16 && strcmp(J.ColorType,'grayscale') && strcmp(J.Format,'DICOM')",
        ],
    );
    // RGB frames: NumberOfFrames and PlanarConfiguration come from X too.
    e.execute(&format!("X=uint8(reshape(mod((0:35)*7,256),2,3,3,2)); dicomwrite(X,'rgb.dcm',struct({SC},'NumberOfFrames',1,'PhotometricInterpretation','MONOCHROME1'),'CreateMode','copy'); J=dicominfo('rgb.dcm'); Y=dicomread('rgb.dcm');")).unwrap();
    all(
        &mut e,
        &[
            "isequal(X,Y) && J.NumberOfFrames==2 && J.PlanarConfiguration==0 && J.SamplesPerPixel==3 && strcmp(J.PhotometricInterpretation,'RGB')",
        ],
    );
    // File meta storage UIDs may come from the struct's own file meta fields.
    e.execute("dicomwrite(uint8(1),'media.dcm',struct('MediaStorageSOPClassUID','1.2.840.10008.5.1.4.1.1.7','MediaStorageSOPInstanceUID','2.25.77','ImplementationVersionName','OTHER'),'CreateMode','copy'); J=dicominfo('media.dcm');").unwrap();
    all(
        &mut e,
        &[
            "strcmp(J.MediaStorageSOPInstanceUID,'2.25.77') && ~isfield(J,'SOPInstanceUID') && ~isfield(J,'SOPClassUID') && strcmp(J.ImplementationVersionName,'MATRUST_7A4')",
        ],
    );
    let before = entries(dir.path());
    for (source, expected) in [
        // Copy mode generates no UIDs.
        (
            "dicomwrite(uint8(1),'bad.dcm',struct('PatientID','x'),'CreateMode','copy')".to_owned(),
            "missing SOPClassUID",
        ),
        (
            "dicomwrite(uint8(1),'bad.dcm',struct('SOPClassUID','1.2.3'),'CreateMode','copy')"
                .to_owned(),
            "missing SOPInstanceUID",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'NotADicomField',1),'CreateMode','copy')"
            ),
            "NotADicomField is not a DICOM attribute name",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'Endian','ieee-be'),'CreateMode','copy')"
            ),
            "Endian is an option",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'OverlayRows',4,'OverlayRows_0',4),'CreateMode','copy')"
            ),
            "another field already supplies",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PatientID',5),'CreateMode','copy')"
            ),
            "PatientID must be a character row vector",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'StudyDate','２０００'),'CreateMode','copy')"
            ),
            "VR DA text must be ASCII",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PatientID',char([65 0 66])),'CreateMode','copy')"
            ),
            "NUL",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SpecificCharacterSet','ISO_IR 100'),'CreateMode','copy')"
            ),
            "SpecificCharacterSet",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SpecificCharacterSet','ISO_IR 6','PatientID','é'),'CreateMode','copy')"
            ),
            "ISO_IR 192",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SelectorUSValue',70000),'CreateMode','copy')"
            ),
            "outside the range of VR US",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SelectorUSValue',1.5),'CreateMode','copy')"
            ),
            "finite integers",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SmallestImagePixelValue',-1),'CreateMode','copy')"
            ),
            "outside the range of VR US",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'AcquisitionNumber',2147483648),'CreateMode','copy')"
            ),
            "VR IS",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PixelSpacing',[1 NaN]),'CreateMode','copy')"
            ),
            "VR DS",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'SelectorFLValue','text'),'CreateMode','copy')"
            ),
            "must be numeric for VR FL",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PatientName',struct('FamilyName','a','Nickname','b')),'CreateMode','copy')"
            ),
            "unknown person-name field 'Nickname'",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PatientName',struct('FamilyName','a^b')),'CreateMode','copy')"
            ),
            "name components",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'ReferencedStudySequence',5),'CreateMode','copy')"
            ),
            "is a sequence",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'ReferencedStudySequence',struct('Item_2',struct())),'CreateMode','copy')"
            ),
            "Item_1 ... Item_n",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'ReferencedStudySequence',struct('Item_1',5)),'CreateMode','copy')"
            ),
            "Item_1 must be a scalar struct",
        ),
        (
            format!(
                "dicomwrite(uint8(1),'bad.dcm',struct({SC},'PatientID',repmat('x',1,65535)),'CreateMode','copy')"
            ),
            "65534-byte limit of VR LO",
        ),
        // Create mode still rejects contradictions in retained pixel fields.
        (
            format!("dicomwrite(uint8(1),'bad.dcm',struct({SC},'Rows',999))"),
            "contradicts the pixel array",
        ),
        (
            format!("dicomwrite(uint8(1),'bad.dcm',struct({SC},'Rows',999),'CreateMode','create')"),
            "contradicts the pixel array",
        ),
        (
            "I=dicominfo('win.dcm'); dicomwrite(uint8(1),'bad.dcm',I)".to_owned(),
            "contradicts the pixel array",
        ),
    ] {
        let error = e.execute(&source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
        assert_eq!(entries(dir.path()), before, "{source}");
    }
    // Implicit VR has 32-bit lengths: the same long LO value is written there.
    e.execute(&format!("dicomwrite(uint8(1),'long.dcm',struct({SC},'PatientID',repmat('x',1,65535)),'CreateMode','copy','VR','implicit'); J=dicominfo('long.dcm'); n=numel(J.PatientID);")).unwrap();
    all(&mut e, &["n==65535"]);
}

#[test]
fn metadata_limits_depth_cancellation_and_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.dcm");
    let mut e = folder(dir.path());
    let roomy = || matrust::engine::Limits {
        memory_bytes: 2048 * 1024 * 1024,
        evaluation_units: None,
        ..Default::default()
    };
    e.limits_set(roomy());
    // Six nested sequence levels are written; the seventh is refused.
    let nested = |levels: usize| {
        let mut value = "struct('CodeValue','deep')".to_owned();
        for _ in 0..levels {
            value = format!("struct('ReferencedStudySequence',struct('Item_1',{value}))");
        }
        format!(
            "D={value}; D.SOPClassUID='1.2.840.10008.5.1.4.1.1.7'; D.SOPInstanceUID='2.25.1234';"
        )
    };
    e.execute(&nested(6)).unwrap();
    e.execute("dicomwrite(uint8(1),'deep.dcm',D,'CreateMode','copy'); J=dicominfo('deep.dcm');")
        .unwrap();
    all(
        &mut e,
        &[
            "strcmp(J.ReferencedStudySequence.Item_1.ReferencedStudySequence.Item_1.ReferencedStudySequence.Item_1.ReferencedStudySequence.Item_1.ReferencedStudySequence.Item_1.ReferencedStudySequence.Item_1.CodeValue,'deep')",
        ],
    );
    std::fs::remove_file(dir.path().join("deep.dcm")).unwrap();
    e.execute(&nested(7)).unwrap();
    // 1 MiB + 1 byte value; 17 one-MiB values (over 16 MiB of metadata).
    let items: Vec<String> = (1..=17).map(|k| format!("'Item_{k}',T")).collect();
    e.execute(&format!("Z=uint8(zeros(1048577,1)); V=struct({SC},'EncapsulatedDocument',Z); T=struct('EncapsulatedDocument',uint8(zeros(1048576,1))); W=struct({SC},'ReferencedStudySequence',struct({})); S=struct({SC},'PatientID','keep');", items.join(","))).unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::write(&path, b"SENTINEL").unwrap();
        }
        let unchanged = |source: &str| {
            if exists {
                assert_eq!(std::fs::read(&path).unwrap(), b"SENTINEL", "{source}");
            } else {
                assert!(!path.exists(), "{source}");
            }
            assert_eq!(entries(dir.path()), usize::from(exists), "{source}");
        };
        for (source, expected) in [
            (
                "dicomwrite(uint8(1),'out.dcm',D,'CreateMode','copy')",
                "exceeds 6 nested sequence levels",
            ),
            (
                "dicomwrite(uint8(1),'out.dcm',V,'CreateMode','copy')",
                "EncapsulatedDocument exceeds the 1 MiB value limit",
            ),
            (
                "dicomwrite(uint8(1),'out.dcm',W,'CreateMode','copy')",
                "exceeds the 16 MiB metadata limit",
            ),
        ] {
            let error = e.execute(source).unwrap_err();
            assert!(error.contains(expected), "{source}: {error}");
            unchanged(source);
        }
        // Memory and evaluation budgets are admitted before any metadata is encoded.
        for limits in [
            matrust::engine::Limits {
                memory_bytes: e.workspace_bytes() + 1024,
                ..roomy()
            },
            matrust::engine::Limits {
                evaluation_units: Some(32),
                ..roomy()
            },
        ] {
            e.limits_set(limits);
            let source = "dicomwrite(uint8(1),'out.dcm',S,'CreateMode','copy')";
            assert!(e.execute(source).is_err());
            unchanged(source);
        }
        e.limits_set(roomy());
        let cancel = e.cancellation();
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        for source in [
            "dicomwrite(uint8(1),'out.dcm',S,'CreateMode','copy')",
            "dicomwrite(uint8(1),'out.dcm',W,'CreateMode','copy')",
        ] {
            let error = e.execute(source).unwrap_err();
            assert!(error.to_lowercase().contains("cancel"), "{source}: {error}");
            unchanged(source);
        }
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    // Just inside the limits: a value of exactly 1 MiB is written whole.
    e.execute(&format!("dicomwrite(uint8(1),'out.dcm',struct({SC},'EncapsulatedDocument',uint8(ones(1048576,1))),'CreateMode','copy');")).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let header = [0x42, 0, 0x11, 0, b'O', b'B', 0, 0, 0, 0, 0x10, 0];
    let at = bytes.windows(12).position(|w| w == header).unwrap();
    assert!(bytes[at + 12..at + 12 + 1048576].iter().all(|b| *b == 1));
    // dicominfo charges every value 16 times against its 16 MiB budget, so it
    // cannot read that file back; a 60000-byte value round-trips.
    assert!(
        e.execute("J=dicominfo('out.dcm');")
            .unwrap_err()
            .contains("budget")
    );
    e.execute(&format!("dicomwrite(uint8(1),'out.dcm',struct({SC},'EncapsulatedDocument',uint8(ones(60000,1))),'CreateMode','copy'); J=dicominfo('out.dcm'); n=numel(J.EncapsulatedDocument);")).unwrap();
    all(&mut e, &["n==60000 && J.EncapsulatedDocument(end)==1"]);
    assert_eq!(entries(dir.path()), 1);
}

#[test]
#[ignore = "manual release-profile copy-mode timings for 1 and N caller workers"]
fn copy_mode_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    pool.install(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut e = folder(dir.path());
        let path = fixture("explicit_le_defined.dcm");
        let large: Vec<String> = (1..=16).map(|k| format!("'Item_{k}',T")).collect();
        e.execute(&format!("X=dicomread('{path}'); T=struct('EncapsulatedDocument',uint8(mod((0:59999)',251)));")).unwrap();
        for (label, source) in [
            ("fixture struct", format!("I=dicominfo('{path}');")),
            (
                "fixture struct + 16 items of 60000 bytes",
                format!("I.ReferencedStudySequence=struct({});", large.join(",")),
            ),
        ] {
            e.execute(&source).unwrap();
            e.execute("dicomwrite(X,'bench.dcm',I,'CreateMode','copy');")
                .unwrap();
            let start = std::time::Instant::now();
            for _ in 0..200 {
                e.execute("dicomwrite(X,'bench.dcm',I,'CreateMode','copy');")
                    .unwrap();
            }
            eprintln!(
                "DICOM copy write {label}, workers={workers}, iterations=200, bytes={}, elapsed_ms={:.3}",
                std::fs::metadata(dir.path().join("bench.dcm"))
                    .unwrap()
                    .len(),
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    });
}

#[test]
#[ignore = "generate current Rust public-call files for independent pydicom inspection"]
fn export_for_pydicom() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom/write_meta");
    std::fs::create_dir_all(&dir).unwrap();
    generate(&dir);
    eprintln!(
        "Generated {} copy-mode DICOM files in {}",
        entries(&dir),
        dir.display()
    );
}

#[test]
fn mixed_signed_lut_descriptors_match_pydicom() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    for source in ["explicit_le", "implicit_le", "explicit_be"] {
        let input = fixture(&format!("mixed_descriptor_{source}.dcm"));
        e.execute(&format!("I=dicominfo('{input}');")).unwrap();
        for field in [
            "ModalityLUTSequence.Item_1.LUTDescriptor",
            "RedPaletteColorLookupTableDescriptor",
            "GreenPaletteColorLookupTableDescriptor",
            "BluePaletteColorLookupTableDescriptor",
        ] {
            e.execute(&format!("v=I.{field};")).unwrap();
            let v = e.array("v").unwrap();
            for (i, want) in [40000., -10., 16.].into_iter().enumerate() {
                assert_eq!(
                    v.scalar(i, 0).unwrap().into_real().unwrap(),
                    want,
                    "{source}: {field}[{i}]"
                );
            }
        }
        // Serialize the descriptors themselves. The existing US/OW fallback
        // for LUTData is separate from the descriptor's mixed signedness.
        e.execute("I.ModalityLUTSequence.Item_1=struct('LUTDescriptor',I.ModalityLUTSequence.Item_1.LUTDescriptor);").unwrap();
        for (k, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!("dicomwrite(int16([1 2 3;4 5 6]),'lut{k}.dcm',I,'CreateMode','copy','TransferSyntax','{syntax}'); J=dicominfo('lut{k}.dcm'); same=isequal(I.ModalityLUTSequence,J.ModalityLUTSequence)&&isequal(I.RedPaletteColorLookupTableDescriptor,J.RedPaletteColorLookupTableDescriptor)&&isequal(I.GreenPaletteColorLookupTableDescriptor,J.GreenPaletteColorLookupTableDescriptor)&&isequal(I.BluePaletteColorLookupTableDescriptor,J.BluePaletteColorLookupTableDescriptor);")).unwrap();
            assert!(logical(&e, "same"));
        }
    }
}

#[test]
fn mixed_signed_lut_descriptor_writer_accepts_unsigned_count() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    for (k, syntax) in SYNTAXES.iter().enumerate() {
        e.execute(&format!("M=struct({SC},'ModalityLUTSequence',struct('Item_1',struct('LUTDescriptor',[40000;-10;16])),'RedPaletteColorLookupTableDescriptor',[40000;-10;16]); dicomwrite(int16(1),'lut{k}.dcm',M,'CreateMode','copy','TransferSyntax','{syntax}');")).unwrap();
        let bytes = std::fs::read(dir.path().join(format!("lut{k}.dcm"))).unwrap();
        let want: &[u8] = if k == 2 {
            &[0x9c, 0x40, 0xff, 0xf6, 0, 16]
        } else {
            &[0x40, 0x9c, 0xf6, 0xff, 16, 0]
        };
        assert!(bytes.windows(6).any(|v| v == want));
    }
}

#[test]
fn mixed_signed_lut_descriptor_component_ranges() {
    // PS3.3 defines unsigned 16-bit first/third and signed 16-bit second cells.
    // This checks representation limits, without claiming clinical LUT validity.
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    for field in [
        "LUTDescriptor",
        "RedPaletteColorLookupTableDescriptor",
        "GreenPaletteColorLookupTableDescriptor",
        "BluePaletteColorLookupTableDescriptor",
    ] {
        for (k, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!("M=struct({SC},'{field}',[65535;-32768;65535]); dicomwrite(int16(1),'range{k}.dcm',M,'CreateMode','copy','TransferSyntax','{syntax}'); J=dicominfo('range{k}.dcm'); ok=isequal(double(J.{field}),[65535;-32768;65535]);")).unwrap();
            assert!(logical(&e, "ok"));
            let bytes = std::fs::read(dir.path().join(format!("range{k}.dcm"))).unwrap();
            let want: &[u8] = if k == 2 {
                &[255, 255, 128, 0, 255, 255]
            } else {
                &[255, 255, 0, 128, 255, 255]
            };
            assert!(contains(&bytes, want));
        }
        for values in [
            "[-1;0;16]",
            "[65536;0;16]",
            "[1;-32769;16]",
            "[1;32768;16]",
            "[1;0;-1]",
            "[1;0;65536]",
        ] {
            let error = e.execute(&format!("M=struct({SC},'{field}',{values}); dicomwrite(int16(1),'bad.dcm',M,'CreateMode','copy');")).unwrap_err();
            assert!(
                error.contains("range of VR SS"),
                "{field} {values}: {error}"
            );
            assert!(!dir.path().join("bad.dcm").exists());
        }
    }
}

#[test]
fn lut_data_us_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    let syntax = SYNTAXES[0];
    e.execute(&format!(
        "M=struct({SC},'ModalityLUTSequence',struct('Item_1',struct('LUTDescriptor',[4;0;16],'LUTData',uint16([100;200;300;65535])))); \
         dicomwrite(int16(1),'lut_us.dcm',M,'CreateMode','copy','TransferSyntax','{syntax}'); \
         J=dicominfo('lut_us.dcm'); \
         ok=isequal(J.ModalityLUTSequence.Item_1.LUTData,uint16([100;200;300;65535]));"
    ))
    .unwrap();
    assert!(logical(&e, "ok"));
    let bytes = std::fs::read(dir.path().join("lut_us.dcm")).unwrap();
    assert!(contains(&bytes, &[0x28, 0x00, 0x06, 0x30, b'U', b'S']));
}

#[test]
fn lut_data_ow_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lut_ow.dcm");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0u8; 128]);
    bytes.extend_from_slice(b"DICM");
    // File Meta: TransferSyntaxUID (0002,0010) = Explicit VR Little Endian (1.2.840.10008.1.2.1)
    let uid = b"1.2.840.10008.1.2.1\0";
    let mut meta = Vec::new();
    meta.extend_from_slice(&0x0002u16.to_le_bytes());
    meta.extend_from_slice(&0x0010u16.to_le_bytes());
    meta.extend_from_slice(b"UI");
    meta.extend_from_slice(&(uid.len() as u16).to_le_bytes());
    meta.extend_from_slice(uid);
    // FileMetaInformationGroupLength (0002,0000) UL
    bytes.extend_from_slice(&0x0002u16.to_le_bytes());
    bytes.extend_from_slice(&0x0000u16.to_le_bytes());
    bytes.extend_from_slice(b"UL");
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&meta);

    // Build Item_1: LUTDescriptor (0028,3002 US) + LUTData (0028,3006 OW)
    let mut item = Vec::new();
    // LUTDescriptor: [4, 0, 16]
    item.extend_from_slice(&0x0028u16.to_le_bytes());
    item.extend_from_slice(&0x3002u16.to_le_bytes());
    item.extend_from_slice(b"US");
    item.extend_from_slice(&6u16.to_le_bytes());
    for v in [4u16, 0, 16] {
        item.extend_from_slice(&v.to_le_bytes());
    }
    // LUTData: [100, 200, 300, 65535] with VR OW (4-byte length in Explicit VR)
    let lut_values = [100u16, 200, 300, 65535];
    item.extend_from_slice(&0x0028u16.to_le_bytes());
    item.extend_from_slice(&0x3006u16.to_le_bytes());
    item.extend_from_slice(b"OW\0\0");
    item.extend_from_slice(&((lut_values.len() * 2) as u32).to_le_bytes());
    for v in lut_values {
        item.extend_from_slice(&v.to_le_bytes());
    }

    // Wrap Item_1 in Item (FFFE,E000) and ModalityLUTSequence (0028,3000 SQ)
    let mut sq_body = Vec::new();
    sq_body.extend_from_slice(&0xFFFEu16.to_le_bytes());
    sq_body.extend_from_slice(&0xE000u16.to_le_bytes());
    sq_body.extend_from_slice(&(item.len() as u32).to_le_bytes());
    sq_body.extend_from_slice(&item);

    bytes.extend_from_slice(&0x0028u16.to_le_bytes());
    bytes.extend_from_slice(&0x3000u16.to_le_bytes());
    bytes.extend_from_slice(b"SQ\0\0");
    bytes.extend_from_slice(&(sq_body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&sq_body);

    std::fs::write(&path, &bytes).unwrap();

    let mut e = folder(dir.path());
    e.execute("J=dicominfo('lut_ow.dcm'); ok=isequal(J.ModalityLUTSequence.Item_1.LUTData,uint16([100;200;300;65535]));")
        .unwrap();
    assert!(logical(&e, "ok"));
}
