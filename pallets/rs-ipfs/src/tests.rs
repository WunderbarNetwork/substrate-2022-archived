use crate::{mock::*, Error};
use frame_support::{assert_noop, assert_ok};
use log::{ info };

/*#[test]
fn it_works_for_default_value() {
	new_test_ext().execute_with(|| {
		// Dispatch a signed extrinsic.
		assert_ok!(TemplateModule::do_something(Origin::signed(1), 42));
		// Read pallet storage and assert an expected result.
		assert_eq!(TemplateModule::something(), Some(42));
	});
}
*/

#[test]
fn it_expects_ipfs_connect_to_add_a_connection() {
	let localhost = vec![127, 0, 0, 1];

	new_test_ext().execute_with(|| {
		assert_ok!(RsIpfs::ipfs_connect(Origin::signed(1), localhost));
		// println!("Value in connection_queue: {:?}", RsIpfs::connection_queue());
		assert_eq!(RsIpfs::connection_queue().unwrap().len(), 1);
	});

}
#[test]
fn it_expects_ipfs_connect_to_have_multiple_connections() {
	let localhost = vec![127, 0, 0, 1];
	let anotherHost = vec![1,1,1,1];

	new_test_ext().execute_with(|| {
		assert_ok!(RsIpfs::ipfs_connect(Origin::signed(1), localhost));
		assert_eq!(RsIpfs::connection_queue().unwrap().len(), 1);

		assert_ok!(RsIpfs::ipfs_connect(Origin::signed(1), anotherHost));
		assert_eq!(RsIpfs::connection_queue().unwrap().len(), 2);
	});
}

#[test]
fn it_tests() {
	new_test_ext().execute_with(|| {
	});
}

/*#[test]
fn correct_error_for_none_value() {
	new_test_ext().execute_with(|| {
		// Ensure the expected error is thrown when no value is present.
		assert_noop!(TemplateModule::cause_error(Origin::signed(1)), Error::<Test>::NoneValue);
	});
}
*/
