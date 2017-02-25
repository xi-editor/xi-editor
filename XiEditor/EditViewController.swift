//
//  EditViewController.swift
//  XiEditor
//
//  Created by Brandon Titus on 11/2/16.
//  Copyright © 2016 Google. All rights reserved.
//

import Cocoa

class EditViewController: NSViewController {

    @IBOutlet var editView: EditView!
    @IBOutlet var shadowView: ShadowView!
    @IBOutlet var scrollView: NSScrollView!
    @IBOutlet var documentView: NSClipView!
    
    override func viewDidLoad() {
        super.viewDidLoad()
        
        self.shadowView.mouseUpHandler = editView.mouseUp
        self.shadowView.mouseDraggedHandler = editView.mouseDragged
        scrollView.contentView.documentCursor = NSCursor.IBeamCursor();
        
        NSNotificationCenter.defaultCenter().addObserver(self, selector: #selector(EditViewController.boundsDidChangeNotification(_:)), name: NSViewBoundsDidChangeNotification, object: scrollView.contentView)
        NSNotificationCenter.defaultCenter().addObserver(self, selector: #selector(EditViewController.frameDidChangeNotification(_:)), name: NSViewFrameDidChangeNotification, object: scrollView)
    }
    
    func boundsDidChangeNotification(notification: NSNotification) {
        updateEditViewScroll()
    }
    
    func frameDidChangeNotification(notification: NSNotification) {
        updateEditViewScroll()
    }
    
    private func updateEditViewScroll() {
        editView?.updateScroll(scrollView.contentView.bounds)
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }
}
