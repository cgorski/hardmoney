//! `hardmoney::parser::webcheck`: request-body shape, response parsing on
//! captured samples, and the diff -- all offline -- plus live tests
//! against the FEC's WebCheck.
//!
//! Network tests are `#[ignore]` and additionally self-skip unless
//! `HARDMONEY_NETWORK_TESTS=1`:
//!
//! ```text
//! HARDMONEY_NETWORK_TESTS=1 cargo test --all-features --test webcheck_oracle -- --ignored --nocapture
//! ```
//!
//! The samples in this file are trimmed copies of what the live service
//! returned on 2026-09-15 for the named fixtures.

#![cfg(feature = "fetch")]

use std::path::{Path, PathBuf};

use hardmoney::Filing;
use hardmoney::parser::webcheck::{
    Credentials, OracleFinding, OracleReport, SOAP_NAMESPACE, WebCheck, WebCheckError,
    base64_encode, diff, field_number, message_matches_rule, multipart_body, parse_report_text,
    parse_soap_response, parse_upload_response, soap_envelope,
};
use hardmoney::parser::{Rule, Severity};

fn fixture_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = fixture_path(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn network_tests_enabled() -> bool {
    if std::env::var("HARDMONEY_NETWORK_TESTS").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping: set HARDMONEY_NETWORK_TESTS=1 to run network tests");
        false
    }
}

// ---------------------------------------------------------------------
// Request bodies (no network)
// ---------------------------------------------------------------------

#[test]
fn base64_matches_rfc4648_and_the_system_encoder() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    // Every byte value, so the whole alphabet and both padding cases are hit.
    let all: Vec<u8> = (0u8..=255).collect();
    let encoded = base64_encode(&all);
    assert_eq!(encoded.len(), 344);
    assert!(encoded.ends_with("+/w=="));
    assert!(encoded.starts_with("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=="));
}

#[test]
fn soap_envelope_has_the_wsdl_shape() {
    let bytes = fixture("F3XA_2011827.fec");
    let env = soap_envelope("key<&>\"", "a@b.example", &bytes);

    assert!(env.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(
        env.contains(
            "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\""
        )
    );
    assert!(env.contains(&format!("xmlns:ser=\"{SOAP_NAMESPACE}\"")));
    assert!(env.contains("<soapenv:Body><ser:validate>"));
    // Document/literal: the three args are unqualified children, in order.
    let a0 = env.find("<arg0>").unwrap();
    let a1 = env.find("<arg1>").unwrap();
    let a2 = env.find("<arg2>").unwrap();
    assert!(a0 < a1 && a1 < a2);
    assert!(
        env.contains("<arg0>key&lt;&amp;&gt;&quot;</arg0>"),
        "XML-escaped"
    );
    assert!(env.contains("<arg1>a@b.example</arg1>"));
    assert!(env.contains(&format!("<arg2>{}</arg2>", base64_encode(&bytes))));
    assert!(env.ends_with("</ser:validate></soapenv:Body></soapenv:Envelope>"));
    // The payload is base64: no raw filing bytes leak into the XML.
    assert!(!env.contains("HDR"));
}

#[test]
fn multipart_body_matches_what_the_webcheck_page_posts() {
    let bytes = fixture("F3XA_2011827.fec");
    let body = multipart_body("----b0undary", "F3XA_2011827.fec", "", &bytes);
    let text = String::from_utf8_lossy(&body);

    assert!(text.starts_with("------b0undary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"F3XA_2011827.fec\"\r\nContent-Type: application/octet-stream\r\n\r\n"));
    assert!(text.contains(
        "\r\n------b0undary\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\n\r\n"
    ));
    assert!(text.ends_with("------b0undary--\r\n"));
    // The file bytes are present verbatim.
    assert!(body.windows(bytes.len()).any(|w| w == &bytes[..]));

    let odd = multipart_body("b", "we\"ird\\na\r\nme.fec", "x@y.z", b"");
    let odd = String::from_utf8_lossy(&odd);
    assert!(odd.contains("filename=\"we\\\"ird\\\\name.fec\""), "{odd}");
    assert!(odd.contains("name=\"email\"\r\n\r\nx@y.z\r\n"));
}

// ---------------------------------------------------------------------
// Response parsing (no network)
// ---------------------------------------------------------------------

/// What `POST /webcheck/services/upload` returned for
/// `tests/fixtures/invalid/bad_dates_and_amounts.fec` on 2026-09-15,
/// trimmed to the parts that matter and with a warnings section added in
/// the same markup (taken from the response for `F3XN_210000_v5.3.fec`).
const SAMPLE_ERRORS_AND_WARNINGS: &str = r##"
<link rel="stylesheet" type="text/css" href="/webcheck/css/custom.css">
<script type="text/javascript">
 	var validationObj = '{"result":"ERRORS","resultURL":""}';
 	var errorsCount = '5';
	var warningsCount = '2';
	var largeFileUploadResponse = '';
	$( document ).ready(function() { var obj = $.parseJSON($.trim(validationObj)); });
</script>
					<div id="erros-warnings">
						<div class="attention" style="text-align: center;">
								<span class='error'>FEC data file FAILED validation with Errors!</span><br><br>
					</div>
					<table class="" style="text-align: left;">
							<tr><td style="width: 25%"> Committee ID:</td><td style="width: 75%">    C00944124</td></tr>
							<tr><td style="width: 25%"> Committee Name:</td><td style="width: 75%">  REVIVE OREGON</td></tr>
							<tr><td style="width: 25%"> Filing Type:</td><td style="width: 75%">     F3XA</td></tr>
					 </table><br>
					<div id="tabs">
					<ul id="tabs-list">
					    <li class="btn primary"><a href="#errors" style="color: #fff;" rel="noopener noreferrer">Errors (5)</a></li>
					    <li class="btn primary"><a href="#warnings" style="color: #fff;" rel="noopener noreferrer">Warnings (2)</a></li>
				   	</ul>
<!-- 				   	<div id="errors-warnings"> -->
					   	<div style="position: relative;" id="errors">
						    	<span class="attention" style="text-align: center; color: #ff0000">Validation Errors must be corrected prior to submitting your report.</span>
								<table>
									<tbody style="width:450px;" class="error-warning-results">
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr>
											  		  	<td width="5%">
											    		1.&nbsp;
											    		</td>
											    		<td>
											    		Form{Item}:  F3XA
											    		</td>
											    	</tr>
											    	<tr>
											    		<td>&nbsp;</td>
											    		<td width="5%">
											    		Field Name:  #022  Treasurer&#039;s Signature Date
											    		</td>
											    	</tr>
											    	<tr>
											    		<td>&nbsp;</td>
											    		<td width="5%">
											    		    20261301 is not a Real Date
											    		</td>
											    	</tr>
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr>
											  		  	<td width="5%">
											    		2.&nbsp;
											    		</td>
											    		<td>
											    		Form{Item}:  SB21B    {Election CFO}
											    		</td>
											    	</tr>
											    	<tr>
											    		<td>&nbsp;</td>
											    		<td width="5%">
											    		Field Name:  #020  Date of Expenditure
											    		</td>
											    	</tr>
											    	<tr>
											    		<td>&nbsp;</td>
											    		<td width="5%">
											    		    20260231 is not a Real Date
											    		</td>
											    	</tr>
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr><td width="5%">3.&nbsp;</td><td>Form{Item}:  SB21B    {Election CFO}</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Field Name:  #020  Date of Expenditure</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Bad Date - 2026-06-22 not YYYYMMDD format</td></tr>
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr><td width="5%">4.&nbsp;</td><td>Form{Item}:  SB21B    {Nelson &amp; Research}</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Field Name:  #021  Amount of Expenditure</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">$5,500.00 not a Valid Amount of Expenditure value</td></tr>
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr><td width="5%">5.&nbsp;</td><td>Form{Item}:  SB21B    {New Media Northwest}</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Field Name:  #021  Amount of Expenditure</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">1500.005 not a Valid Amount of Expenditure value</td></tr>
									</tbody>
								</table>
						</div>
					   	<div style="position: relative;" id="warnings">
								<table>
									<tbody style="width:450px;" class="error-warning-results">
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr><td width="5%">1.&nbsp;</td><td>Form{Item}:  SA11A1    {Flowood}</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Field Name:  #017  Contributor ZIP Code</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">Zip Code is Invalid or Missing / Zip = 15</td></tr>
													<tr><td width="5%">&nbsp;</td><td>&nbsp;</td></tr>
											  		<tr><td width="5%">2.&nbsp;</td><td>Form{Item}:  F3XN</td></tr>
											    	<tr><td>&nbsp;</td><td width="5%">A message with no field cell</td></tr>
									</tbody>
								</table>
						</div>
<!-- 					</div> -->
				</div>
			</div>
"##;

/// What the upload channel returned for `tests/fixtures/F3XA_2011827.fec`.
const SAMPLE_SUCCESS: &str = r##"
<script type="text/javascript">
 	var validationObj = '{"result":"SUCCESS","resultURL":""}';
 	var errorsCount = '0';
	var warningsCount = '0';
	var largeFileUploadResponse = '';
</script>
					<div id="erros-warnings">
					<table class="" style="text-align: left;">
							<tr><td style="width: 25%"> Committee ID:</td><td style="width: 75%">    C00944124</td></tr>
							<tr><td style="width: 25%"> Filing Type:</td><td style="width: 75%">     F3XA</td></tr>
					 </table><br>
					<div id="tabs">
					<ul id="tabs-list">
					    <li class="btn primary"><a href="#errors">Errors (0)</a></li>
					    <li class="btn primary"><a href="#warnings">Warnings (0)</a></li>
				   	</ul>
					   	<div style="position: relative;" id="errors">
						</div>
					   	<div style="position: relative;" id="warnings">
						</div>
				</div>
			</div>
"##;

/// What the upload channel returned for `tests/fixtures/F3A_2004471.fec`
/// on 2026-09-15 (trimmed): a `WARNINGS` verdict, which the page renders
/// as "FEC data file PASSED validation with Warnings!".
const SAMPLE_WARNINGS_ONLY: &str = r##"
<script type="text/javascript">
 	var validationObj = '{"result":"WARNINGS","resultURL":""}';
 	var errorsCount = '0';
	var warningsCount = '2';
	var largeFileUploadResponse = '';
</script>
					<div id="erros-warnings">
						<div class="attention" style="text-align: center;">
								<span class='warning'>FEC data file PASSED validation with Warnings!</span><br><br>
					</div>
					<table class="" style="text-align: left;">
							<tr><td style="width: 25%"> Committee ID:</td><td style="width: 75%">    C00837567</td></tr>
							<tr><td style="width: 25%"> Filing Type:</td><td style="width: 75%">     F3A</td></tr>
					 </table><br>
					<div id="tabs">
					   	<div style="position: relative;" id="errors">
						</div>
					   	<div style="position: relative;" id="warnings">
						    	<span class="attention">Validation Warning messages are not required to be corrected in order to file your report.</span>
								<table><tbody class="error-warning-results">
									<tr><td width="5%">1.&nbsp;</td><td>Form{Item}:  SB17     {FRAMER}</td></tr>
									<tr><td>&nbsp;</td><td>Field Name:  #016  Recipient State Code</td></tr>
									<tr><td>&nbsp;</td><td>is Required, but field is Empty</td></tr>
									<tr><td width="5%">2.&nbsp;</td><td>Form{Item}:  SB17     {FRAMER}</td></tr>
									<tr><td>&nbsp;</td><td>Field Name:  #017  Recipient ZIP Code</td></tr>
									<tr><td>&nbsp;</td><td>Zip Code is Invalid or Missing / Zip = 1016</td></tr>
								</tbody></table>
						</div>
				</div>
			</div>
"##;

/// What `POST /webcheck/services/validate` returned for every request
/// without a valid vendor API key on 2026-09-15 (MTOM `multipart/related`
/// wrapping the SOAP envelope; the `<return>` string is JSON).
const SAMPLE_SOAP_REJECTED: &str = "\r\n--uuid:23ba9206-b583-48b0-844b-d6e0667c0a3e\r\nContent-Type: application/xop+xml; charset=UTF-8; type=\"text/xml\"\r\nContent-Transfer-Encoding: binary\r\nContent-ID: <root.message@cxf.apache.org>\r\n\r\n<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><ns2:validateResponse xmlns:ns2=\"http://service.webcheck.efo.fec.gov/\"><return>{\"status\":\"FAILED\",\"msg_url\":\"\",\"message\":\"Error! API Key is invalid. To obtain a lost or forgotten API key, please visit Vendor Registration at: https://efilingapps.fec.gov/registration/vendorlogin.htm. For further assistance, Please contact the FEC Electronic Filing Office for assistance at (202) 694-1307.\",\"email\":\"\",\"submission_id\":\"\",\"batch_id\":0,\"success\":false}</return></ns2:validateResponse></soap:Body></soap:Envelope>\r\n--uuid:23ba9206-b583-48b0-844b-d6e0667c0a3e--";

/// What the SOAP service returned (HTTP 500) for a body whose element
/// is not in the WSDL.
const SAMPLE_SOAP_FAULT: &str = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body><soap:Fault><faultcode>soap:Client</faultcode><faultstring>Message part nope was not recognized.  (Does it exist in service WSDL?)</faultstring></soap:Fault></soap:Body></soap:Envelope>"#;

#[test]
fn parses_errors_and_warnings_sections() {
    let report = parse_upload_response(SAMPLE_ERRORS_AND_WARNINGS).unwrap();
    assert_eq!(report.result.as_deref(), Some("ERRORS"));
    assert_eq!(report.errors_reported, Some(5));
    assert_eq!(report.warnings_reported, Some(2));
    assert_eq!(report.filing_type.as_deref(), Some("F3XA"));
    assert_eq!(report.committee_id.as_deref(), Some("C00944124"));
    assert!(!report.is_acceptable());
    assert_eq!(report.error_count(), 5);
    assert_eq!(report.warning_count(), 2);
    assert!(!report.counts_disagree());

    let f = &report.findings[0];
    assert_eq!(f.severity, Some(Severity::Error));
    assert_eq!(
        f.line_no, None,
        "the upload channel reports no line numbers"
    );
    assert_eq!(f.form_type.as_deref(), Some("F3XA"));
    assert_eq!(f.item, None);
    assert_eq!(f.field_no, Some(22));
    assert_eq!(
        f.field_label.as_deref(),
        Some("Treasurer's Signature Date"),
        "&#039; decoded"
    );
    assert_eq!(f.message, "20261301 is not a Real Date");

    let f = &report.findings[1];
    assert_eq!(f.form_type.as_deref(), Some("SB21B"));
    assert_eq!(f.item.as_deref(), Some("Election CFO"));
    assert_eq!(f.field_no, Some(20));
    assert_eq!(f.field_label.as_deref(), Some("Date of Expenditure"));
    assert_eq!(f.message, "20260231 is not a Real Date");

    assert_eq!(
        report.findings[2].message,
        "Bad Date - 2026-06-22 not YYYYMMDD format"
    );
    assert_eq!(
        report.findings[3].item.as_deref(),
        Some("Nelson & Research"),
        "&amp; decoded"
    );
    assert_eq!(
        report.findings[3].message,
        "$5,500.00 not a Valid Amount of Expenditure value"
    );
    assert_eq!(report.findings[4].field_no, Some(21));

    let w = &report.findings[5];
    assert_eq!(w.severity, Some(Severity::Warning));
    assert_eq!(w.form_type.as_deref(), Some("SA11A1"));
    assert_eq!(w.item.as_deref(), Some("Flowood"));
    assert_eq!(w.field_no, Some(17));
    assert_eq!(w.message, "Zip Code is Invalid or Missing / Zip = 15");

    let w = &report.findings[6];
    assert_eq!(w.form_type.as_deref(), Some("F3XN"));
    assert_eq!(w.field_no, None);
    assert_eq!(w.field_label, None);
    assert_eq!(w.message, "A message with no field cell");

    assert_eq!(report.raw, SAMPLE_ERRORS_AND_WARNINGS);

    let text = report.to_string();
    assert!(text.contains(
        "ERROR SB21B #020 Date of Expenditure {Election CFO}: 20260231 is not a Real Date\n"
    ));
    assert!(text.contains("WARN  SA11A1 #017 Contributor ZIP Code {Flowood}: Zip Code"));
    assert!(text.ends_with(
        "WebCheck NOT ACCEPTABLE (ERRORS): 5 error(s), 2 warning(s), filing type F3XA\n"
    ));
}

#[test]
fn parses_a_clean_response() {
    let report = parse_upload_response(SAMPLE_SUCCESS).unwrap();
    assert_eq!(report.result.as_deref(), Some("SUCCESS"));
    assert_eq!(report.errors_reported, Some(0));
    assert_eq!(report.warnings_reported, Some(0));
    assert!(report.findings.is_empty());
    assert!(report.is_acceptable());
    assert_eq!(
        report.to_string(),
        "WebCheck ACCEPTABLE: 0 error(s), 0 warning(s), filing type F3XA\n"
    );
}

/// A `WARNINGS` verdict is an accepted filing: the FEC's own page says
/// "PASSED validation with Warnings", and its message list says a warning
/// "will not prevent the filing from being processed".
#[test]
fn warnings_only_is_acceptable() {
    let report = parse_upload_response(SAMPLE_WARNINGS_ONLY).unwrap();
    assert_eq!(report.result.as_deref(), Some("WARNINGS"));
    assert_eq!(report.errors_reported, Some(0));
    assert_eq!(report.warnings_reported, Some(2));
    assert_eq!(report.warning_count(), 2);
    assert!(!report.counts_disagree());
    assert!(report.is_acceptable());
    assert_eq!(report.filing_type.as_deref(), Some("F3A"));
    let zip = &report.findings[1];
    assert_eq!(zip.severity, Some(Severity::Warning));
    assert_eq!(zip.form_type.as_deref(), Some("SB17"));
    assert_eq!(zip.field_no, Some(17));
    assert_eq!(zip.message, "Zip Code is Invalid or Missing / Zip = 1016");
    assert!(message_matches_rule(Rule::InvalidZipCode, &zip.message));
    assert!(
        report.to_string().ends_with(
            "WebCheck ACCEPTABLE (WARNINGS): 0 error(s), 2 warning(s), filing type F3A\n"
        ),
        "{report}"
    );
    // WebCheck words a blank `X (warning)` column as a required field; the
    // live alternate lets the diff pair it with our `recommended_field_empty`.
    assert_eq!(
        report.findings[0].message,
        "is Required, but field is Empty"
    );
    assert!(message_matches_rule(
        Rule::RecommendedFieldEmpty,
        &report.findings[0].message
    ));
    assert!(message_matches_rule(
        Rule::RecommendedFieldEmpty,
        "Street Address is Missing"
    ));
}

#[test]
fn count_disagreement_is_flagged_not_hidden() {
    let tampered = SAMPLE_SUCCESS.replace("errorsCount = '0'", "errorsCount = '3'");
    let report = parse_upload_response(&tampered).unwrap();
    assert!(report.counts_disagree());
    assert!(
        report
            .to_string()
            .contains("response format may have changed")
    );
}

#[test]
fn unexpected_bodies_are_errors_not_panics() {
    match parse_upload_response("<html><body>Service Unavailable</body></html>") {
        Err(WebCheckError::Unparseable { snippet }) => {
            assert!(snippet.contains("Service Unavailable"))
        }
        other => panic!("{other:?}"),
    }
    match parse_upload_response("") {
        Err(WebCheckError::Unparseable { .. }) => {}
        other => panic!("{other:?}"),
    }
    // The page's own error path: a JSON body.
    match parse_upload_response(r#"{"result":"Only .fec files are accepted"}"#) {
        Err(WebCheckError::Rejected { message }) => {
            assert_eq!(message, "Only .fec files are accepted")
        }
        other => panic!("{other:?}"),
    }
    // Over 20 MB: the results go by e-mail.
    let deferred = SAMPLE_SUCCESS.replace(
        "largeFileUploadResponse = ''",
        "largeFileUploadResponse = 'Results will be emailed to <b>x@y.z</b>'",
    );
    match parse_upload_response(&deferred) {
        Err(WebCheckError::Deferred { message }) => {
            assert_eq!(message, "Results will be emailed to x@y.z");
        }
        other => panic!("{other:?}"),
    }
    // Sections present but empty of cells, and a cell soup with no
    // numbering: still a report, never a panic.
    let soup = r#"validationObj = '{"result":"ERRORS"}' <div id="errors"><td>orphan text</td><td>Form{Item}: X</td></div>"#;
    let report = parse_upload_response(soup).unwrap();
    assert_eq!(report.result.as_deref(), Some("ERRORS"));
    assert!(report.findings.is_empty());
}

#[test]
fn plain_text_reports_keep_each_line() {
    let report =
        parse_report_text("ERROR: line 4: Something\n\nWarning - Something else\n").unwrap();
    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.findings[0].severity, Some(Severity::Error));
    assert_eq!(report.findings[0].line_no, Some(4));
    assert_eq!(report.findings[0].message, "Something");
    assert_eq!(report.findings[1].severity, Some(Severity::Warning));
    assert_eq!(report.findings[1].line_no, None);
    assert_eq!(report.findings[1].message, "Something else");
    assert_eq!(report.result, None);
    assert!(!report.is_acceptable());

    // An HTML fragment handed to the text parser is recognised as such.
    let report = parse_report_text(SAMPLE_SUCCESS).unwrap();
    assert_eq!(report.result.as_deref(), Some("SUCCESS"));

    assert!(matches!(
        parse_report_text("  \n"),
        Err(WebCheckError::Unparseable { .. })
    ));
}

#[test]
fn soap_return_json_is_decoded_from_the_mtom_wrapper() {
    let ret = parse_soap_response(SAMPLE_SOAP_REJECTED).unwrap();
    assert_eq!(ret.status, "FAILED");
    assert!(ret.message.starts_with("Error! API Key is invalid."));
    assert_eq!(ret.msg_url, "");
    assert_eq!(ret.submission_id, "");
    assert_eq!(ret.batch_id, 0);
    assert!(!ret.success);
    assert!(ret.raw.starts_with('{'));

    // A hypothetical accepted answer.
    let ok = r#"<soap:Envelope xmlns:soap="x"><soap:Body><ns2:validateResponse xmlns:ns2="y"><return>{"status":"ACCEPTED","msg_url":"https://example.test/r.txt","message":"","email":"a@b.c","submission_id":"abc-123","batch_id":42,"success":true}</return></ns2:validateResponse></soap:Body></soap:Envelope>"#;
    let ret = parse_soap_response(ok).unwrap();
    assert!(ret.success);
    assert_eq!(ret.batch_id, 42);
    assert_eq!(ret.submission_id, "abc-123");
    assert_eq!(ret.msg_url, "https://example.test/r.txt");
    assert_eq!(ret.email, "a@b.c");

    // Entity-escaped JSON (a non-MTOM server would write it this way).
    let escaped = r#"<return>{&quot;status&quot;:&quot;X&quot;,&quot;success&quot;:true}</return>"#;
    let ret = parse_soap_response(escaped).unwrap();
    assert_eq!(ret.status, "X");
    assert!(ret.success);
}

#[test]
fn soap_faults_and_garbage_are_typed_errors() {
    match parse_soap_response(SAMPLE_SOAP_FAULT) {
        Err(WebCheckError::SoapFault { code, string }) => {
            assert_eq!(code, "soap:Client");
            assert!(string.starts_with("Message part nope was not recognized."));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse_soap_response("<html>nope</html>"),
        Err(WebCheckError::Unparseable { .. })
    ));
    assert!(matches!(
        parse_soap_response("<return>not json</return>"),
        Err(WebCheckError::Unparseable { .. })
    ));
    let e = parse_soap_response(SAMPLE_SOAP_FAULT).unwrap_err();
    assert_eq!(
        e.to_string(),
        "WebCheck SOAP fault soap:Client: Message part nope was not recognized.  (Does it exist in service WSDL?)"
    );
}

// ---------------------------------------------------------------------
// Diff (no network)
// ---------------------------------------------------------------------

#[test]
fn field_numbers_are_the_spec_column_plus_one() {
    let ours = Filing::open(fixture_path("invalid/bad_dates_and_amounts.fec"))
        .unwrap()
        .validate();
    let numbers: Vec<(String, Option<&str>, Option<u16>)> = ours
        .iter()
        .map(|f| (f.form_type.clone(), f.field, field_number(f)))
        .collect();
    assert_eq!(
        numbers,
        [
            ("F3XA".to_string(), Some("date_signed"), Some(22)),
            ("SB21B".to_string(), Some("expenditure_date"), Some(20)),
            ("SB21B".to_string(), Some("expenditure_date"), Some(20)),
            ("SB21B".to_string(), Some("expenditure_amount"), Some(21)),
            ("SB21B".to_string(), Some("expenditure_amount"), Some(21)),
        ]
    );

    // HDR is not in the form-type dispatch table but has a spec.
    let old = Filing::open(fixture_path("F3XN_210000_v5.3.fec"))
        .unwrap()
        .validate();
    let hdr = old.iter().find(|f| f.rule == Rule::CurrentFormat).unwrap();
    assert_eq!(hdr.form_type, "HDR");
    assert_eq!(field_number(hdr), Some(3));
}

#[test]
fn diff_matches_by_form_field_and_template() {
    let ours = Filing::open(fixture_path("invalid/bad_dates_and_amounts.fec"))
        .unwrap()
        .validate();
    let theirs = parse_upload_response(SAMPLE_ERRORS_AND_WARNINGS).unwrap();
    let d = diff(&ours, &theirs);

    assert_eq!(d.matched.len(), 5, "{d}");
    assert!(d.only_ours.is_empty(), "{d}");
    // The two warnings in the sample belong to another filing.
    assert_eq!(d.only_theirs.len(), 2, "{d}");
    assert!(!d.is_empty());
    assert_eq!(d.severity_disagreements().count(), 0);

    for (ours, theirs) in &d.matched {
        assert_eq!(Some(ours.form_type.as_str()), theirs.form_type.as_deref());
        assert_eq!(field_number(ours), theirs.field_no);
        assert!(message_matches_rule(ours.rule, &theirs.message));
    }
    // Order is ours' file order; each of theirs used once.
    assert_eq!(d.matched[0].0.line_no, 2);
    assert_eq!(d.matched[4].0.line_no, 7);
    assert_eq!(d.matched[3].1.item.as_deref(), Some("Nelson & Research"));
    assert_eq!(d.matched[4].1.item.as_deref(), Some("New Media Northwest"));

    let text = d.to_string();
    assert!(text.starts_with("Matched (5):\n"));
    assert!(text.contains("\nOnly ours (0):\nOnly theirs (2):\n"));
    assert!(text.ends_with("5 matched, 0 only ours, 2 only theirs\n"));
}

#[test]
fn diff_against_a_clean_report_is_all_only_ours() {
    let ours = Filing::open(fixture_path("invalid/duplicate_tran_id.fec"))
        .unwrap()
        .validate();
    let theirs = parse_upload_response(SAMPLE_SUCCESS).unwrap();
    let d = diff(&ours, &theirs);
    assert!(d.matched.is_empty());
    assert_eq!(d.only_ours.len(), ours.len());
    assert!(d.only_theirs.is_empty());

    let empty = diff(
        &Filing::open(fixture_path("F3XA_2011827.fec"))
            .unwrap()
            .validate(),
        &theirs,
    );
    assert!(empty.is_empty());
    assert_eq!(
        empty.to_string(),
        "Matched (0):\nOnly ours (0):\nOnly theirs (0):\n0 matched, 0 only ours, 0 only theirs\n"
    );
}

#[test]
fn diff_notes_a_severity_disagreement() {
    let ours = Filing::open(fixture_path("F3XN_210000_v5.3.fec"))
        .unwrap()
        .validate();
    let mut theirs = OracleReport::default();
    let mut f = OracleFinding::from_message("Filing Format must be Version 8.5");
    f.severity = Some(Severity::Error);
    f.form_type = Some("HDR".to_string());
    f.field_no = Some(3);
    f.field_label = Some("FEC Version#".to_string());
    theirs.findings.push(f);

    let d = diff(&ours, &theirs);
    assert_eq!(d.matched.len(), 1);
    assert_eq!(d.matched[0].0.rule, Rule::CurrentFormat);
    assert_eq!(d.severity_disagreements().count(), 1);
    assert!(d.to_string().contains("[severity differs]"));
}

#[test]
fn credentials_redact_the_key_in_debug() {
    let c = Credentials::new("sekrit").with_email("a@b.c");
    let shown = format!("{c:?}");
    assert!(!shown.contains("sekrit"));
    assert_eq!(c.api_key, "sekrit");
    assert_eq!(c.email.as_deref(), Some("a@b.c"));
}

#[test]
fn client_urls_follow_the_endpoint() {
    let wc = WebCheck::new();
    assert_eq!(
        wc.upload_url(),
        "https://efoservices.fec.gov/webcheck/services/upload"
    );
    assert_eq!(
        wc.soap_url(),
        "https://efoservices.fec.gov/webcheck/services/validate"
    );
    let wc = WebCheck::with_endpoint("http://localhost:8080/webcheck/");
    assert_eq!(
        wc.upload_url(),
        "http://localhost:8080/webcheck/services/upload"
    );
}

// ---------------------------------------------------------------------
// Live service
// ---------------------------------------------------------------------

/// Every rule an `invalid/` fixture is pinned to, and the WebCheck
/// message(s) the live service returned for it on 2026-09-15.
const INVALID_FIXTURES: &[&str] = &[
    "invalid/amendment_missing_ids.fec",
    "invalid/bad_dates_and_amounts.fec",
    "invalid/bad_filer_id.fec",
    "invalid/dangling_back_reference.fec",
    "invalid/duplicate_tran_id.fec",
    "invalid/field_too_long.fec",
    "invalid/filer_id_mismatch.fec",
    "invalid/illegal_character.fec",
    "invalid/multi_form.fec",
    "invalid/wrong_schedule_for_form.fec",
];

#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn live_accepted_fixture_passes_webcheck() {
    if !network_tests_enabled() {
        return;
    }
    let bytes = fixture("F3XA_2011827.fec");
    let report = WebCheck::new()
        .submit("F3XA_2011827.fec", &bytes, None)
        .unwrap();
    eprintln!("--- F3XA_2011827.fec ---\n{report}");
    assert_eq!(report.result.as_deref(), Some("SUCCESS"), "{}", report.raw);
    assert_eq!(report.errors_reported, Some(0));
    assert_eq!(report.warnings_reported, Some(0));
    assert!(report.findings.is_empty());
    assert_eq!(report.filing_type.as_deref(), Some("F3XA"));
    assert!(report.is_acceptable());

    let ours = Filing::parse_bytes(&bytes).unwrap().validate();
    let d = diff(&ours, &report);
    eprintln!("{d}");
    assert!(d.is_empty(), "{d}");
}

/// The 2026 party, House, JFC, and presidential fixtures against the live
/// service (observed 2026-09-15). Two state-party reports with Schedules
/// H2-H4 and the joint fundraising committee come back `SUCCESS` with no
/// messages -- so WebCheck reads the `NUM-5` allocation percentages
/// (`0.49`) as numeric, as hardmoney now does. The House amendment draws
/// WebCheck's `Zip Code is Invalid or Missing / Zip = 1016` (W30) and, for
/// its blank payee state, `is Required, but field is Empty` at warning
/// severity (the workbook marks the column `X (warning)`); both pair with
/// ours. The Georgia report draws three `Conditionally Required field is
/// Empty` warnings
/// (donor committee id on a `PTY` line; candidate id and last name on a
/// `CCM` line), all paired with ours; we additionally warn on the `CCM`
/// line's blank candidate office, which the workbook marks `Used if CAN or
/// CCM` and WebCheck does not report.
#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn live_party_and_candidate_fixtures_agree_with_webcheck() {
    if !network_tests_enabled() {
        return;
    }
    let wc = WebCheck::new();
    for (name, warnings, matched, only_ours, only_theirs) in [
        ("F3XN_1998773.fec", 0, 0, 0, 0),
        ("F3XA_2008083.fec", 0, 0, 0, 0),
        ("F3XN_1965568.fec", 0, 0, 0, 0),
        ("F3A_2004471.fec", 2, 2, 0, 0),
        ("F3XA_2011814.fec", 3, 3, 1, 0),
    ] {
        let bytes = fixture(name);
        let report = wc.submit(name, &bytes, None).unwrap();
        let ours = Filing::parse_bytes(&bytes).unwrap().validate();
        let d = diff(&ours, &report);
        eprintln!("--- {name} ---\n{ours}{report}{d}");
        assert_eq!(report.errors_reported, Some(0), "{name}: {}", report.raw);
        assert_eq!(report.warnings_reported, Some(warnings), "{name}");
        assert!(
            !report.counts_disagree(),
            "{name}: response format changed?"
        );
        assert!(report.is_acceptable(), "{name}");
        assert!(ours.is_acceptable(), "{name}: {ours}");
        assert_eq!(d.matched.len(), matched, "{name}: {d}");
        assert_eq!(d.only_ours.len(), only_ours, "{name}: {d}");
        assert_eq!(d.only_theirs.len(), only_theirs, "{name}: {d}");
        for (ours, theirs) in &d.matched {
            assert_eq!(
                theirs.severity,
                Some(ours.severity),
                "{name}: {ours} ~ {theirs}"
            );
        }
    }
}

#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn live_invalid_fixtures_fail_webcheck() {
    if !network_tests_enabled() {
        return;
    }
    let wc = WebCheck::new();
    for rel in INVALID_FIXTURES {
        let bytes = fixture(rel);
        let name = Path::new(rel).file_name().unwrap().to_string_lossy();
        let report = wc.submit(&name, &bytes, None).unwrap();
        let ours = Filing::parse_bytes(&bytes).unwrap().validate();
        let d = diff(&ours, &report);
        eprintln!("--- {rel} ---\n{ours}{report}{d}");

        assert_eq!(
            report.result.as_deref(),
            Some("ERRORS"),
            "{rel}: {}",
            report.raw
        );
        assert!(report.error_count() >= 1, "{rel}");
        assert!(!report.counts_disagree(), "{rel}: response format changed?");
        assert!(!report.is_acceptable(), "{rel}");
        for f in &report.findings {
            assert!(f.form_type.is_some(), "{rel}: {f}");
            assert!(f.field_no.is_some(), "{rel}: {f}");
            assert!(!f.message.is_empty(), "{rel}: {f}");
        }
        // Where hardmoney and WebCheck agree on the rule and the field,
        // the diff pairs them. Files where the two validators attribute the
        // defect differently are documented in book/src/validating.md.
        match *rel {
            "invalid/amendment_missing_ids.fec" => {
                // We: two HDR errors (#17, #9); they: one F3XA "Header (HDR)
                // inconsistent with Orig/Amend status" (#8).
                assert_eq!(d.matched.len(), 0, "{d}");
                assert_eq!(d.only_theirs.len(), 1, "{d}");
            }
            "invalid/multi_form.fec" => {
                // Agree on the empty treasurer name; they pin the multi-form
                // error on the cover and also reject the extra F3XN as a
                // schedule, we pin it on the offending line.
                assert_eq!(d.matched.len(), 1, "{d}");
                assert_eq!(d.only_ours.len(), 1, "{d}");
                assert_eq!(d.only_theirs.len(), 2, "{d}");
            }
            _ => {
                assert!(d.only_ours.is_empty(), "{rel}: {d}");
                assert!(d.only_theirs.is_empty(), "{rel}: {d}");
                assert_eq!(d.matched.len(), ours.len(), "{rel}: {d}");
            }
        }
    }
}

#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn live_soap_channel_requires_a_vendor_api_key() {
    if !network_tests_enabled() {
        return;
    }
    let bytes = fixture("F3XA_2011827.fec");
    let creds = Credentials::new("not-a-real-key").with_email("nobody@example.com");
    let err = WebCheck::new()
        .submit("F3XA_2011827.fec", &bytes, Some(&creds))
        .unwrap_err();
    eprintln!("--- SOAP validate with a bogus key ---\n{err}");
    match err {
        WebCheckError::Rejected { message } => {
            assert!(message.contains("API Key is invalid"), "{message}");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}
