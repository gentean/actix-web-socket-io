use std::{collections::HashMap, sync::Arc};

use crossbeam::channel::{self, Sender};

use lazy_static::lazy_static;
use serde::Serialize;
use tokio::{
    sync::RwLock,
    time::{sleep, Duration},
};

#[derive(Debug, Clone, Serialize)]
pub struct ValData {
    pub id: i32,
    pub value: i32,
}

lazy_static! {
    // 所有的监听回调
    static ref CHANGE_HANDLERS: RwLock<HashMap<i32, Vec<Sender<Arc<ValData>>>>> = RwLock::new(HashMap::new());
}

/// 监听 id 值变化
pub async fn watch_change(ids: Vec<i32>) -> (channel::Receiver<Arc<ValData>>, impl FnOnce()) {
    let (sender, recv) = channel::unbounded();
    let mut change_handlers = CHANGE_HANDLERS.write().await;

    // 加入点号回调
    for id in ids.clone() {
        if let Some(handlers) = change_handlers.get_mut(&id) {
            handlers.push(sender.clone());
        } else {
            change_handlers.insert(id, vec![sender.clone()]);
        }
    }

    (recv, move || {
        // 取消监听逻辑
        tokio::spawn(async move {
            let mut change_handlers = CHANGE_HANDLERS.write().await;
            for id in ids {
                if let Some(handlers) = change_handlers.get_mut(&id) {
                    handlers.retain(|item| sender.same_channel(item) == false);
                }
            }
            drop(sender);
        });
    })
}

/// 触发给订阅了这个 id 的上层
async fn trigger_dit_change(id: i32, val_data: ValData) {
    let share_val_data = Arc::new(val_data);
    let change_handlers = CHANGE_HANDLERS.read().await;
    if let Some(handlers) = change_handlers.get(&id) {
        handlers.iter().for_each(|sender| {
            if let Err(msg) = sender.send(share_val_data.clone()) {
                log::error!("广播消息失败 msg={:?}, val_data={:?}", msg, share_val_data);
            }
        });
    }
}

/// 生成变动数据
pub async fn gen_change_data() -> anyhow::Result<()> {
    // 清空监听回调差释放锁
    {
        let mut change_handlers = CHANGE_HANDLERS.write().await;
        change_handlers.clear();
    }

    tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(300)).await;
            let change_handlers = CHANGE_HANDLERS.read().await;

            for id in change_handlers.keys() {
                let data = ValData { id: *id, value: 1 };
                trigger_dit_change(*id, data).await;
            }
        }
    });

    Ok(())
}
