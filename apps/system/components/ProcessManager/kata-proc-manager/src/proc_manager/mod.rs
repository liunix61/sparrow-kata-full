//! Kata OS process management support

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use core::convert::TryFrom;
use core::marker::Sync;
use hashbrown::HashMap;
use log::trace;
use smallstr::SmallString;

use kata_proc_common as proc;
use proc::*;

pub type BundleId = SmallString<[u8; DEFAULT_BUNDLE_ID_CAPACITY]>;

// Bundle capacity before spillover to the heap.
pub const DEFAULT_BUNDLES_CAPACITY: usize = 10;

// Bundle state tracks start/stop operations.
#[derive(Debug, Eq, PartialEq)]
enum BundleState {
    Stopped,
    Running,
}

// We track the Bundle & ProcessControlInterface state.
struct BundleData {
    state: BundleState,
    bundle: Box<Bundle>,
    // TODO(sleffler): seL4 capability (minted from bundle.app_token)?
    // TODO(sleffler): seL4 capability for bundle.app_memory_address?
}

impl BundleData {
    fn new(bundle: &Bundle) -> Self {
        BundleData {
            state: BundleState::Stopped,
            bundle: Box::new(bundle.clone()),
        }
    }
}

// The ProcessManager presents the PackageManagementInterface (for loading
// applications from storage) and the ProcessControlInterface (for starting
// and stopping associated applications). The interface to the underlying
// system(s) are abstracted through the ProcessManagerInterface. One instance
// of the ProcessManager is created at start and accessed through SeL4 RPC's
// (from other components).
pub struct ProcessManager {
    manager: Box<dyn ProcessManagerInterface + Sync>,
    bundles: HashMap<BundleId, BundleData>,
}

impl ProcessManager {
    // Creates a new ProcessManager instance.
    pub fn new(manager: impl ProcessManagerInterface + Sync + 'static) -> ProcessManager {
        ProcessManager {
            manager: Box::new(manager),
            bundles: HashMap::with_capacity(DEFAULT_BUNDLES_CAPACITY),
        }
    }

    pub fn capacity(&self) -> usize {
        self.bundles.capacity()
    }
}

impl PackageManagementInterface for ProcessManager {
    // NB: doc says a bundle may have multiple apps; support one for now
    //   (assume a fixed pathname to the app is used)
    fn install(
        &mut self,
        pkg_buffer: *const u8,
        pkg_buffer_len: usize,
    ) -> Result<String, ProcessManagerError> {
        let pkg_buffer_size = match u32::try_from(pkg_buffer_len) {
            Ok(v) => v,
            Err(_) => return Err(ProcessManagerError::PackageBufferLenInvalid),
        };

        // We assume the seL4 capability for the memory associated with
        // pkg_buffer has been setup for us so we can pass it along (as needed)
        // to the StorageManager.
        //
        // NB: defer to StorageManager for handling an install of a previously
        // installed app. We do not have the app_id to check locally so if the
        // StorageManager disallows re-install then we'll return it's error;
        // otherwise we update the returned Bundle state.
        // TODO(sleffler): owner's public key?
        let bundle = self.manager.install(pkg_buffer, pkg_buffer_size)?;
        trace!(
            "install pkg {:p}:{} => bundle_id:{}",
            pkg_buffer,
            pkg_buffer_size,
            &bundle.app_id
        );

        assert!(self
            .bundles
            .insert(BundleId::from_str(&bundle.app_id), BundleData::new(&bundle))
            .is_none());

        Ok(bundle.app_id)
    }

    fn uninstall(&mut self, bundle_id: &str) -> Result<(), ProcessManagerError> {
        trace!("uninstall bundle_id:{}", bundle_id);
        let bid = BundleId::from_str(bundle_id);
        match self.bundles.get(&bid) {
            Some(bundle) => {
                trace!(
                    "uninstall bundle_id:{}: state {:?}",
                    bundle_id,
                    bundle.state
                );
                if bundle.state == BundleState::Running {
                    return Err(ProcessManagerError::BundleRunning);
                }
                let _ = self.bundles.remove(&bid);
            }
            None => {}
        }
        // NB: the hashmap is ephemeral so always call through to the manager
        self.manager.uninstall(bundle_id)
    }
}

impl ProcessControlInterface for ProcessManager {
    fn start(&mut self, bundle_id: &str) -> Result<(), ProcessManagerError> {
        let bid = BundleId::from_str(bundle_id);
        match self.bundles.get_mut(&bid) {
            Some(bundle) => {
                trace!("start bundle_id:{}: state {:?}", bundle_id, bundle.state);
                if bundle.state == BundleState::Stopped {
                    self.manager.start(&bundle.bundle)?;
                }
                bundle.state = BundleState::Running;
                Ok(())
            }
            None => {
                // We depend on the hashmap contents since we need the Bundle
                // to setup/start the application. To that end we pre-populate
                // the hashmap at start by querying the StorageManager for
                // previously installed applications.
                trace!("start bundle_id:{}: not found", bundle_id);
                Err(ProcessManagerError::BundleNotFound)
            }
        }
    }

    fn stop(&mut self, bundle_id: &str) -> Result<(), ProcessManagerError> {
        let bid = BundleId::from_str(bundle_id);
        match self.bundles.get_mut(&bid) {
            Some(bundle) => {
                trace!("stop bundle_id:{}: state {:?}", bundle_id, bundle.state);
                if bundle.state == BundleState::Running {
                    self.manager.stop(&bundle.bundle)?;
                }
                bundle.state = BundleState::Stopped;
                Ok(())
            }
            None => {
                trace!("stop bundle_id:{}: not found", bundle_id);
                Err(ProcessManagerError::BundleNotFound)
            }
        }
    }

    fn get_running_bundles(&self) -> Result<BundleIdArray, ProcessManagerError> {
        trace!("get_running_bundles");
        let mut result = BundleIdArray::new();
        for (bundle_id, _bundle) in self
            .bundles
            .iter()
            .filter(|(_, bundle)| matches!(bundle.state, BundleState::Running))
        {
            result.push(String::from(bundle_id.as_str()));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc::ProcessManagerError as pme;

    // NB: just enough state to track install'd bundles
    struct FakeManager {
        bundles: HashMap<String, u32>, // pkg_buffer:pkg_buffer_size
    }
    impl FakeManager {
        pub fn new() -> Self {
            FakeManager {
                bundles: HashMap::new(),
            }
        }
    }
    impl ProcessManagerInterface for FakeManager {
        fn install(&mut self, pkg_buffer: *const u8, pkg_buffer_size: u32) -> Result<Bundle, pme> {
            let str = (pkg_buffer as usize).to_string();
            if self.bundles.contains_key(&str) {
                return Err(ProcessManagerError::BundleFound);
            }
            assert!(self.bundles.insert(str, pkg_buffer_size).is_none());
            Ok(Bundle {
                app_id: (pkg_buffer as usize).to_string(),
                data: [0u8; 64],
            })
        }
        fn uninstall(&mut self, bundle_id: &str) -> Result<(), pme> {
            match self.bundles.remove(bundle_id) {
                Some(_) => Ok(()),
                None => Err(ProcessManagerError::BundleNotFound),
            }
        }
        fn start(&mut self, bundle: &Bundle) -> Result<(), pme> {
            assert!(self.bundles.contains_key(&bundle.app_id));
            Ok(())
        }
        fn stop(&mut self, bundle: &Bundle) -> Result<(), pme> {
            assert!(self.bundles.contains_key(&bundle.app_id));
            Ok(())
        }
    }

    #[test]
    fn test_bundle_id_basics() {
        let bundle_id = BundleId::new();
        assert_eq!(bundle_id.len(), 0);
        assert_eq!(bundle_id.inline_size(), DEFAULT_BUNDLE_ID_CAPACITY);

        // Check str conversion.
        assert_eq!(BundleId::from_str("hello").as_str(), "hello");
    }

    #[test]
    fn test_pkg_mgmt() {
        let fake = tests::FakeManager::new();
        let mut mgr = ProcessManager::new(fake);

        // Not installed, should fail.
        assert_eq!(mgr.uninstall("foo").err(), Some(pme::BundleNotFound));

        // Install the bundle.
        let pkg_buffer = [0u8; 1024];
        let result = mgr.install(pkg_buffer.as_ptr(), pkg_buffer.len());
        assert!(result.is_ok());
        let bundle_id = result.unwrap();

        // Re-install the same bundle should fail.
        assert_eq!(
            mgr.install(pkg_buffer.as_ptr(), pkg_buffer.len()).err(),
            Some(pme::BundleFound)
        );

        // Verify you cannot uninstall a running bundle.
        assert!(mgr.start(&bundle_id).is_ok());
        assert_eq!(mgr.uninstall(&bundle_id).err(), Some(pme::BundleRunning));
        assert!(mgr.stop(&bundle_id).is_ok());

        // Now uninstalling the bundle should work.
        assert!(mgr.uninstall(&bundle_id).is_ok());
    }

    #[test]
    fn test_spill() {
        let fake = tests::FakeManager::new();
        let mut mgr = ProcessManager::new(fake);
        let pkg_buffer = [0u8; 1024];

        for i in 0..=mgr.capacity() {
            let slice = &pkg_buffer[i..];
            assert!(mgr.install(slice.as_ptr(), slice.len()).is_ok());
        }
    }

    #[test]
    fn test_proc_ctrl() {
        let fake = tests::FakeManager::new();
        let mut mgr = ProcessManager::new(fake);

        fn is_running(running: &BundleIdArray, id: &str) -> bool {
            running.as_slice().iter().find(|&x| *x == id).is_some()
        }

        let pkg_buffer2 = [0u8; 1024];
        let result2 = mgr.install(pkg_buffer2.as_ptr(), pkg_buffer2.len());
        assert!(result2.is_ok());
        let bid2 = result2.unwrap();

        let pkg_buffer9 = [0u8; 1024];
        let result9 = mgr.install(pkg_buffer9.as_ptr(), pkg_buffer9.len());
        assert!(result9.is_ok());
        let bid9 = result9.unwrap();

        assert!(mgr.stop(&bid2).is_ok());
        assert!(mgr.start(&bid2).is_ok());
        assert!(mgr.start(&bid9).is_ok());

        let running = mgr.get_running_bundles().unwrap();
        assert_eq!(running.len(), 2);
        assert!(is_running(&running, &bid2));
        assert!(is_running(&running, &bid9));

        assert!(mgr.stop(&bid2).is_ok());
        // After stopping the bundle we should see nothing running.
        let running = mgr.get_running_bundles().unwrap();
        assert_eq!(running.len(), 1);
        assert!(is_running(&running, &bid9));

        assert!(mgr.stop(&bid9).is_ok());
        // After stopping the bundle we should see nothing running.
        let running = mgr.get_running_bundles().unwrap();
        assert_eq!(running.len(), 0);
    }

    // TODO(sleffler): check uninstall stops a running thread
}
